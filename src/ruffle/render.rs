//! SWF → PDF rendering (v1/v2 pipelines).
//!
//! Receives patched SWF file paths and renders them through the single
//! persistent GPU worker defined in [`crate::ruffle::exporter`]. Pages arrive
//! as already-JPEG-encoded byte buffers, so this module only has to:
//!   1. queue jobs grouped by SWF (sysb content first, then masks),
//!   2. forward each page to the shared PDF writer in [`crate::pdf`].
//!
//! Crucially there is **no disk round-trip**: the previous implementation wrote
//! every page to a temp JPEG file and re-read it for oxidize-pdf. That doubled
//! I/O and forced a temp dir lifetime to bracket the whole run.

use std::path::PathBuf;

use anyhow::{Context, Result};
use image::DynamicImage;

use crate::cli::Files;
use crate::image_proc::UpscaleOpts;
use crate::pdf::{PageInput, PdfOutput, write_pages};
use crate::ruffle::exporter::{Exporter, RenderedPage};

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

/// Stream `swf_inputs` (sorted sysb-first) into a single PDF.
///
/// Because Ruffle/WGPU owns a GPU context that cannot be shared across threads,
/// all rendering happens on one worker. Throughput is improved instead by doing
/// the JPEG encoding on that same worker (see [`crate::ruffle::exporter`]) and
/// by writing the PDF incrementally via [`crate::pdf::write_pages`].
pub fn render(
    exporter: &Exporter,
    swf_inputs: &[SwfInput],
    file_info: &Files,
    upscale: &UpscaleOpts,
) -> Result<()> {
    let worker = exporter.spawn_worker();

    // Queue every SWF up-front; the worker processes them one at a time on its
    // own thread. Order is preserved, so sysb (content) precedes sysm (mask).
    let job_rx_list: Vec<_> = swf_inputs
        .iter()
        .map(|input| {
            tracing::info!(
                swf = %input.name,
                dims = format!("{}x{}", input.width as u32, input.height as u32),
                "queued for render"
            );
            worker
                .send_job(input.path.clone())
                .with_context(|| format!("queueing render job for {}", input.name))
        })
        .collect::<Result<Vec<_>>>()?;

    let total_swf = swf_inputs.len();
    let mut total_pages = 0u32;
    let pages = PageStream {
        receivers: job_rx_list,
        swf_names: swf_inputs.iter().map(|s| s.name.clone()).collect(),
        swf_index: 0,
        cur_failures: 0,
        total_swf,
    };

    let out = PdfOutput {
        path: file_info.output.clone(),
        title: file_info.filename.clone(),
    };
    write_pages(
        pages.filter_map(|res| match res {
            Ok(page) => {
                total_pages += 1;
                Some(PageInput::new(decode_jpeg_as_image(&page.jpeg)))
            }
            Err(e) => {
                tracing::warn!(error = %e, "frame dropped");
                None
            }
        }),
        &out,
        upscale,
    )?;

    // Shut down the worker (job_tx is dropped → worker loop ends). A join
    // error is only logged, not fatal — the PDF is already fully written.
    if let Err(e) = worker.join() {
        tracing::warn!(error = ?e, "render worker join error");
    }

    tracing::info!(pages = total_pages, "render PDF finished");
    Ok(())
}

/// Decode the JPEG bytes produced by the render worker back into a
/// `DynamicImage` for the shared `pdf` module (which re-embeds JPEG).
///
/// This round-trip exists only because `pdf.rs` works on `DynamicImage` to stay
/// uniform across the v1/v2/v3 paths. The cost is small relative to the GPU
/// render; a single decode failure yields a 1×1 blank page rather than aborting
/// the whole multi-hundred-page book.
fn decode_jpeg_as_image(jpeg: &[u8]) -> DynamicImage {
    match image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg) {
        Ok(img) => img,
        Err(e) => {
            tracing::warn!(error = %e, "failed to re-decode worker JPEG; emitting blank");
            DynamicImage::ImageRgb8(image::RgbImage::new(1, 1))
        }
    }
}

/// Lazy iterator over [`RenderedPage`]s produced by the worker, advancing
/// through the queued SWFs one at a time.
struct PageStream {
    receivers: Vec<std::sync::mpsc::Receiver<Result<RenderedPage>>>,
    swf_names: Vec<String>,
    swf_index: usize,
    cur_failures: u32,
    #[allow(dead_code)]
    total_swf: usize,
}

impl Iterator for PageStream {
    type Item = Result<RenderedPage>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.swf_index >= self.receivers.len() {
                return None;
            }
            let rx = &self.receivers[self.swf_index];
            match rx.recv() {
                Ok(Ok(page)) => return Some(Ok(page)),
                Ok(Err(e)) => {
                    self.cur_failures += 1;
                    return Some(Err(e));
                }
                Err(std::sync::mpsc::RecvError) => {
                    // This SWF's job is done; advance to the next.
                    let name = &self.swf_names[self.swf_index];
                    if self.cur_failures > 0 {
                        tracing::warn!(
                            swf = %name,
                            dropped = self.cur_failures,
                            "SWF had failed/dropped frames"
                        );
                    } else {
                        tracing::debug!(swf = %name, "SWF stream complete");
                    }
                    self.cur_failures = 0;
                    self.swf_index += 1;
                }
            }
        }
    }
}
