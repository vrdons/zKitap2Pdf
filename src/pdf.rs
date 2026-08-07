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
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageFormat};
use oxidize_pdf::{Document, Image, Page};

use crate::image_proc::{merge_overlay, to_rgb};

/// Metadata applied to every generated PDF.
pub struct PdfOutput {
    /// Final destination path (e.g. `out/book.pdf`).
    pub path: std::path::PathBuf,
    /// Document title (usually the book/file stem).
    pub title: String,
}

/// Incremental PDF writer: pages are appended one at a time and the document
/// is only saved (atomically) on [`finish`](PdfWriter::finish). Decoupling the
/// `Document` from the page loops lets v3 parallelise decode/encode while the
/// writer thread stays the single owner of the `Document`.
pub struct PdfWriter<'a> {
    doc: Document,
    out: &'a PdfOutput,
    count: u32,
}

impl<'a> PdfWriter<'a> {
    pub fn new(out: &'a PdfOutput) -> Self {
        let mut doc = Document::new();
        doc.set_title(&out.title);
        doc.set_author(crate::config::PDF_AUTHOR);
        Self { doc, out, count: 0 }
    }

    /// Append a pre-encoded JPEG page (dimensions parsed from the header).
    pub fn add_jpeg_page(&mut self, jpeg: &[u8]) -> Result<()> {
        let (w, h) = match jpeg_dimensions(jpeg) {
            Some((w, h)) => (w, h),
            None => {
                tracing::warn!(page = self.count + 1, "unreadable JPEG header, skipping");
                return Ok(());
            }
        };
        self.add_jpeg_with_dims(jpeg, w, h)
    }

    /// Append a pre-encoded JPEG page whose pixel dimensions are already known
    /// (e.g. from the render worker), skipping the header parse entirely.
    pub fn add_jpeg_with_dims(&mut self, jpeg: &[u8], width: u32, height: u32) -> Result<()> {
        add_jpeg_page(&mut self.doc, jpeg, width as f64, height as f64)?;
        self.count += 1;
        self.log_progress();
        Ok(())
    }

    fn log_progress(&self) {
        if self.count.is_multiple_of(25) {
            tracing::debug!(pages = self.count, "pages written");
        }
    }

    /// Save the document atomically. Errors if no pages were added (matches
    /// the previous behaviour of refusing to emit an empty PDF).
    pub fn finish(mut self) -> Result<()> {
        if self.count == 0 {
            bail!(
                "no pages to write for {} (refusing to emit an empty PDF)",
                self.out.path.display()
            );
        }
        save_atomic(&mut self.doc, &self.out.path)
            .with_context(|| format!("saving PDF to {}", self.out.path.display()))?;
        tracing::info!(pages = self.count, output = %self.out.path.display(), "PDF written");
        Ok(())
    }
}

pub trait PageSink {
    fn add_jpeg_page(&mut self, jpeg: &[u8]) -> Result<()>;

    fn add_jpeg_with_dims(&mut self, jpeg: &[u8], _width: u32, _height: u32) -> Result<()> {
        self.add_jpeg_page(jpeg)
    }

    fn finish(self: Box<Self>) -> Result<()>;
}

impl PageSink for PdfWriter<'_> {
    fn add_jpeg_page(&mut self, jpeg: &[u8]) -> Result<()> {
        PdfWriter::add_jpeg_page(self, jpeg)
    }

    fn add_jpeg_with_dims(&mut self, jpeg: &[u8], width: u32, height: u32) -> Result<()> {
        PdfWriter::add_jpeg_with_dims(self, jpeg, width, height)
    }

    fn finish(self: Box<Self>) -> Result<()> {
        (*self).finish()
    }
}

pub struct ExportWriter {
    dir: PathBuf,
    stem: String,
    count: u32,
}

impl ExportWriter {
    pub fn new(dir: &Path, stem: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            stem: stem.to_string(),
            count: 0,
        }
    }
}

impl PageSink for ExportWriter {
    fn add_jpeg_page(&mut self, jpeg: &[u8]) -> Result<()> {
        self.count += 1;
        let name = format!("{}_{:04}.jpg", self.stem, self.count);
        let path = self.dir.join(name);
        fs::write(&path, jpeg).with_context(|| format!("writing {}", path.display()))?;
        tracing::info!(page = self.count, path = %path.display(), "page exported");
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<()> {
        if self.count == 0 {
            bail!(
                "no pages to export into {} (refusing empty export)",
                self.dir.display()
            );
        }
        tracing::info!(
            pages = self.count,
            dir = %self.dir.display(),
            "JPEG export finished"
        );
        Ok(())
    }
}

/// Parse JPEG dimensions from the SOF marker without decoding the image.
///
/// Returns `None` on malformed input (e.g. progressive JPEGs with a SOF2
/// marker still carry the same header layout, so this handles all common
/// variants).
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    // SOI (FF D8) → scan segments for SOF0..SOF15 (skip DHT/DAC etc.).
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None; // lost sync
        }
        let marker = data[i + 1];
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if marker == 0xC0 || marker == 0xC1 || marker == 0xC2 || marker == 0xC3 {
            // SOF: height (2) width (2) after precision byte.
            if i + 9 > data.len() {
                return None;
            }
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]);
            return Some((w as u32, h as u32));
        }
        if len < 2 || i + 2 + len > data.len() {
            return None;
        }
        i += 2 + len;
    }
    None
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
/// RGB8 sources are passed through without copying. This is the single
/// normalisation point used by every pipeline before asking oxidize-pdf to
/// embed the bytes.
pub fn image_to_jpeg(img: &DynamicImage) -> Result<Vec<u8>> {
    let rgb = to_rgb(img);
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
        Some(layer) => DynamicImage::ImageRgb8(merge_overlay(to_rgb(&base), layer)),
        None => base,
    }
}
