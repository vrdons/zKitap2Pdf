//! Shared PDF writer used by every pipeline (v1/v2 SWF render and v3 webp).
//!
//! Design goals:
//! * Never hold all page images in RAM — pages are pushed one at a time and
//!   JPEG-encoded *before* being handed to oxidize-pdf, which embeds the
//!   compressed DCT stream directly (no intermediate decode).
//! * Atomic output: write to a sibling temp file then rename on success, so a
//!   failure never leaves a half-written/broken `.pdf` masquerading as the real
//!   result.
//! * Consistent metadata + page sizing across all input formats.
//!
//! All image → JPEG conversion goes through [`image_to_jpeg`], which normalises
//! WebP, PNG, RGB and RGBA sources to a JPEG byte buffer acceptable by
//! [`oxidize_pdf::Image::from_jpeg_data`].

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageFormat};
use oxidize_pdf::{Document, Image, Page};

use crate::image_proc::{merge_overlay, upscale_image};

/// Metadata applied to every generated PDF.
pub struct PdfOutput {
    /// Final destination path (e.g. `out/book.pdf`).
    pub path: std::path::PathBuf,
    /// Document title (usually the book/file stem).
    pub title: String,
}

/// A single page ready to be appended to a PDF.
///
/// Decoupling the source (`DynamicImage`) from the writer lets the caller
/// decode webp blobs, merge overlays and upscale in any order before handing
/// the page over — without the writer needing to know about webp/layers.
pub struct PageInput {
    pub image: DynamicImage,
}

impl PageInput {
    pub fn new(image: DynamicImage) -> Self {
        Self { image }
    }
}

/// Build a PDF from an iterator of pages, writing it atomically.
///
/// `pages` is consumed lazily; each page is JPEG-encoded and embedded
/// immediately, so peak memory is roughly one page image at a time.
///
/// If `pages` yields nothing, this returns `Ok(())` without creating a file
/// rather than emitting an invalid zero-page PDF. Callers that want to treat
/// an empty book as an error should check beforehand.
pub fn write_pages<I>(
    pages: I,
    out: &PdfOutput,
    opts: &crate::image_proc::UpscaleOpts,
) -> Result<()>
where
    I: IntoIterator<Item = PageInput>,
{
    let mut doc = Document::new();
    doc.set_title(&out.title);
    doc.set_author(crate::config::PDF_AUTHOR);

    let mut count = 0u32;
    for page in pages {
        let img = upscale_image(page.image, opts);
        let (w, h) = (img.width() as f64, img.height() as f64);

        let jpeg =
            image_to_jpeg(&img).with_context(|| format!("encode page {} to JPEG", count + 1))?;
        add_jpeg_page(&mut doc, &jpeg, w, h)?;
        count += 1;

        if count.is_multiple_of(25) {
            tracing::debug!(pages = count, "pages written");
        }
    }

    if count == 0 {
        bail!(
            "no pages to write for {} (refusing to emit an empty PDF)",
            out.path.display()
        );
    }

    save_atomic(&mut doc, &out.path)
        .with_context(|| format!("saving PDF to {}", out.path.display()))?;
    tracing::info!(pages = count, output = %out.path.display(), "PDF written");
    Ok(())
}

/// Append one JPEG-encoded page to a document.
///
/// `width`/`height` are in PDF points; for our use they equal the image's
/// pixel dimensions, which makes the image fill the page 1:1 (no margins).
fn add_jpeg_page(doc: &mut Document, jpeg: &[u8], width: f64, height: f64) -> Result<()> {
    let pdf_image = Image::from_jpeg_data(jpeg.to_vec())
        .context("oxidize-pdf failed to parse generated JPEG")?;
    let mut page = Page::new(width, height);
    page.add_image("img", pdf_image);
    page.draw_image("img", 0.0, 0.0, width, height)
        .context("draw_image failed")?;
    doc.add_page(page);
    Ok(())
}

/// Encode any `image`-decodable image to a JPEG byte buffer.
///
/// WebP and RGBA inputs are converted to RGB8 first (JPEG has no alpha channel).
/// This is the single normalisation point used by every pipeline before asking
/// oxidize-pdf to embed the bytes.
pub fn image_to_jpeg(img: &DynamicImage) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut buf = Vec::with_capacity((rgb.width() as usize) * (rgb.height() as usize) / 4);
    rgb.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
        .context("JPEG encode failed")?;
    Ok(buf)
}

/// Save a document to `dest` atomically: write to `<dest>.tmp` then rename.
///
/// This guarantees readers never observe a truncated/half-written PDF if the
/// process dies mid-write (e.g. OOM during a large book, or a signal).
fn save_atomic(doc: &mut Document, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output dir {}", parent.display()))?;
    }

    let mut tmp = dest.to_path_buf();
    let mut stem = dest
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    stem.push(".partial");
    tmp.set_file_name(stem);

    doc.save(&tmp)
        .with_context(|| format!("writing temp {}", tmp.display()))?;

    fs::rename(&tmp, dest)
        .with_context(|| format!("renaming {} → {}", tmp.display(), dest.display()))?;
    Ok(())
}

/// Convenience: merge an optional RGBA overlay onto an RGB page, returning a
/// new `DynamicImage`. Thin wrapper around [`merge_overlay`] so callers don't
/// need to import `image_proc` directly for the common v3 layer case.
pub(crate) fn page_with_overlay(
    base: DynamicImage,
    overlay: Option<&DynamicImage>,
) -> DynamicImage {
    match overlay {
        Some(layer) => DynamicImage::ImageRgb8(merge_overlay(base.to_rgb8(), layer)),
        None => base,
    }
}
