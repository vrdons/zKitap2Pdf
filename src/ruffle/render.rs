//! SWF → PDF rendering: drives Ruffle's frame capture and assembles the PDF.

use std::fs;
use std::io::{Cursor, Seek, SeekFrom, Write};

use anyhow::{Context, Result};
use image::{DynamicImage, ImageFormat};
use oxidize_pdf::{Document, Image, Page};
use tempfile::NamedTempFile;

use crate::cli::Files;
use crate::config::PDF_AUTHOR;
use crate::ruffle::exporter::Exporter;
use crate::ruffle::swf;

/// Render the supplied (name, fws-bytes) SWF batches into a single PDF.
pub fn render(
    exporter: &Exporter,
    swf_data: &[(String, Vec<u8>)],
    file_info: &Files,
    scale: f64,
) -> Result<()> {
    let mut doc = Document::new();
    doc.set_title(&file_info.filename);
    doc.set_author(PDF_AUTHOR);

    // One reusable JPEG scratch file across all pages — overwritten per frame.
    let mut jpeg_file = NamedTempFile::new().context("create jpeg scratch file")?;
    let mut total_pages = 0u32;

    for (idx, (name, fws_bytes)) in swf_data.iter().enumerate() {
        // Single decompress per SWF: load() returns both the buffer (for patch)
        // and the metadata (size + frame count).
        let (swf_buf, view) = swf::load(fws_bytes).with_context(|| format!("analysing {name}"))?;
        let patched = swf::patch(swf_buf, view.width, view.height)?;

        let patched_file = NamedTempFile::new().context("create patched swf scratch file")?;
        fs::write(patched_file.path(), &patched)
            .with_context(|| format!("writing patched swf {name}"))?;

        let scaled_width = (view.width * scale).round();
        let scaled_height = (view.height * scale).round();
        let frame_count = view.frame_count as u32;
        tracing::info!(
            index = idx + 1,
            total = swf_data.len(),
            swf = %name,
            scale = format!("{scaled_width:.0}x{scaled_height:.0}"),
            frames = frame_count,
            "processing"
        );

        let mut rendered = 0u32;
        exporter.capture_frames(patched_file.path(), |_frame_idx, image| match capture_page(
            &image,
            &mut jpeg_file,
            &mut doc,
            scaled_width,
            scaled_height,
        ) {
            Ok(()) => rendered += 1,
            Err(e) => tracing::warn!(frame = rendered + 1, error = %e, "page dropped"),
        })?;

        total_pages += rendered;
        tracing::info!(pages = rendered, swf = %name, "captured");
    }

    drop(jpeg_file);

    tracing::info!(pages = total_pages, "finished");
    tracing::info!(output = %file_info.output.display(), "writing PDF");

    if let Some(parent) = file_info.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output dir {}", parent.display()))?;
    }
    doc.save(&file_info.output)
        .with_context(|| format!("saving {}", file_info.output.display()))?;

    Ok(())
}

/// Encode one RGBA frame as JPEG into the scratch file and append a PDF page.
fn capture_page(
    image: &image::RgbaImage,
    jpeg_file: &mut NamedTempFile,
    doc: &mut Document,
    width: f64,
    height: f64,
) -> Result<()> {
    let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();

    let mut jpeg_buf = Cursor::new(Vec::with_capacity(rgb.len() / 3));
    rgb.write_to(&mut jpeg_buf, ImageFormat::Jpeg)
        .context("encode JPEG")?;
    let jpeg_bytes = jpeg_buf.into_inner();

    jpeg_file
        .as_file()
        .set_len(0)
        .and_then(|()| jpeg_file.as_file().seek(SeekFrom::Start(0)))
        .context("reset jpeg scratch")?;
    jpeg_file
        .write_all(&jpeg_bytes)
        .context("write jpeg scratch")?;
    jpeg_file.as_file().flush().context("flush jpeg scratch")?;

    let pdf_image = Image::from_jpeg_file(jpeg_file.path()).context("parse jpeg for pdf")?;

    let mut page = Page::new(width, height);
    page.add_image("img", pdf_image);
    if let Err(e) = page.draw_image("img", 0.0, 0.0, width, height) {
        tracing::warn!(error = %e, "pdf draw_image");
    }
    doc.add_page(page);
    Ok(())
}
