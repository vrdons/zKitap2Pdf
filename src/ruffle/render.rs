//! SWF → PDF rendering: receives patched SWF file paths from the pipeline,
//! spawns all render threads in parallel, then drains frames by SWF order
//! (sysb → sysm → ...) so JPEG page order is correct.
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use image::{DynamicImage, ImageFormat, RgbaImage};
use oxidize_pdf::{Document, Image, Page};
use tempfile::TempDir;

use crate::cli::Files;
use crate::config::PDF_AUTHOR;
use crate::ruffle::exporter::Exporter;

/// A render job spawned in a background thread.
struct RenderJob {
    pub name: String,
    pub handle: JoinHandle<()>,
    pub rx: mpsc::Receiver<Result<(u16, RgbaImage)>>,
    pub scaled_width: f64,
    pub scaled_height: f64,
}

/// Metadata for a single patched SWF ready to render.
pub struct SwfInput {
    /// Display name (e.g. "sysb.dll").
    pub name: String,
    /// Path to the patched FWS file on disk.
    pub path: PathBuf,
    /// Real pixel width (after patch).
    pub width: f64,
    /// Real pixel height (after patch).
    pub height: f64,
}

/// Render the supplied patched SWF files into a single PDF.
///
/// All render threads are spawned **in parallel** (so WGPU can pipeline work
/// across SWFs), then the main thread drains each channel in the original
/// `swf_inputs` order.  Because `swf_inputs` is sorted sysb-first, pages
/// from the content SWF always precede mask pages in the PDF.
pub fn render(
    exporter: &Exporter,
    swf_inputs: &[SwfInput],
    file_info: &Files,
    scale: f64,
) -> Result<()> {
    let mut doc = Document::new();
    doc.set_title(&file_info.filename);
    doc.set_author(PDF_AUTHOR);

    let tmp = TempDir::new().context("create render temp dir")?;
    let mut jpg_seq: u64 = 0;

    // -----------------------------------------------------------------
    // Phase 1 — spawn all render threads at once (parallel GPU work)
    // -----------------------------------------------------------------
    let mut jobs: Vec<RenderJob> = Vec::with_capacity(swf_inputs.len());
    for (idx, input) in swf_inputs.iter().enumerate() {
        let scaled_width = (input.width * scale).round();
        let scaled_height = (input.height * scale).round();

        tracing::info!(
            index = idx + 1,
            total = swf_inputs.len(),
            swf = %input.name,
            scale = format!("{scaled_width:.0}x{scaled_height:.0}"),
            "processing"
        );

        let thread_id = idx as u32 + 1;
        let (handle, rx) = exporter
            .capture_frames_threaded(&input.path, thread_id)
            .with_context(|| format!("spawning render thread for {}", input.name))?;

        jobs.push(RenderJob {
            name: input.name.clone(),
            handle,
            rx,
            scaled_width,
            scaled_height,
        });
    }

    // -----------------------------------------------------------------
    // Phase 2 — drain channels in order → JPEG → PDF pages
    // -----------------------------------------------------------------
    let mut total_pages = 0u32;
    for job in jobs {
        let scaled_width = job.scaled_width;
        let scaled_height = job.scaled_height;

        let mut rendered = 0u32;
        for received in job.rx {
            match received {
                Ok((_frame_idx, rgba)) => {
                    let jpg_path = tmp.path().join(format!("p_{:08}.jpg", jpg_seq));
                    jpg_seq += 1;
                    save_jpeg(&rgba, &jpg_path)?;
                    drop(rgba);
                    add_pdf_page(&mut doc, &jpg_path, scaled_width, scaled_height)?;
                    rendered += 1;
                }
                Err(e) => {
                    tracing::warn!(swf = %job.name, error = %e, "frame dropped");
                }
            }
        }

        // Join the thread — it should already be finished by the time the
        // channel is exhausted, but we wait here for correctness.
        job.handle
            .join()
            .map_err(|e| anyhow!("render thread '{}' panicked: {e:?}", job.name))?;

        total_pages += rendered;
        tracing::info!(pages = rendered, swf = %job.name, "captured");
    }

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

/// Encode a single RGBA frame as a JPEG file on disk.
fn save_jpeg(rgba: &image::RgbaImage, path: &Path) -> Result<()> {
    let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
    let file = fs::File::create(path)
        .with_context(|| format!("creating {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    rgb.write_to(&mut writer, ImageFormat::Jpeg)
        .context("encode JPEG")?;
    writer.flush().context("flush JPEG")?;
    Ok(())
}

/// Read a JPEG from disk and append it as a PDF page.
fn add_pdf_page(doc: &mut Document, jpg_path: &Path, width: f64, height: f64) -> Result<()> {
    let pdf_image = Image::from_jpeg_file(jpg_path)
        .with_context(|| format!("parse jpeg for pdf: {}", jpg_path.display()))?;

    let mut page = Page::new(width, height);
    page.add_image("img", pdf_image);
    if let Err(e) = page.draw_image("img", 0.0, 0.0, width, height) {
        tracing::warn!(error = %e, "pdf draw_image");
    }
    doc.add_page(page);
    Ok(())
}
