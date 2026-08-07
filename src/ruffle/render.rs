//! SWF → PDF rendering (v1/v2 pipelines).
//!
//! Receives patched SWF file paths and renders them through the single
//! persistent GPU worker defined in [`crate::ruffle::exporter`]. Pages arrive
//! as already-JPEG-encoded byte buffers, so this module only has to:
//!   1. queue jobs grouped by SWF (sysb content first, then masks),
//!   2. forward each page to the shared PDF writer in [`crate::pdf`].
//!
//! Crucially there is **no disk round-trip and no decode/re-encode round-trip**:
//! the worker's JPEG bytes are embedded directly (upscaling already happened
//! at capture time via Ruffle's viewport scale).

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cli::Files;
use crate::pdf::PdfOutput;
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
/// by writing the PDF incrementally via [`crate::pdf::PdfWriter`].
///
/// Note: the upscale stage is deliberately *not* applied here — Ruffle already
/// renders at the configured scale (`--scale`), so a second upscale in the PDF
/// writer would double the size (3.24× instead of 1.8×) for zero quality gain.
pub fn render(exporter: &Exporter, swf_inputs: &[SwfInput], file_info: &Files) -> Result<()> {
    let worker = exporter.spawn_worker();

    let job_scales: Vec<f64> = match exporter.target_dpi() {
        Some(dpi) => {
            let factor = dpi as f64 / 72.0;
            let min_w = swf_inputs
                .iter()
                .map(|s| s.width)
                .fold(f64::INFINITY, f64::min);
            let min_h = swf_inputs
                .iter()
                .map(|s| s.height)
                .fold(f64::INFINITY, f64::min);
            let (tw, th) = (min_w * factor, min_h * factor);
            tracing::info!(
                dpi,
                min_page = format!("{}x{}", min_w as u32, min_h as u32),
                target = format!("{}x{}", tw as u32, th as u32),
                "target-DPI render (per-page scale, no downscale)"
            );
            swf_inputs
                .iter()
                .map(|s| {
                    if s.width >= tw && s.height >= th {
                        1.0 // already at/above target — render as-is
                    } else {
                        factor
                    }
                })
                .collect()
        }
        None => swf_inputs.iter().map(|_| exporter.scale()).collect(),
    };

    // Queue every SWF up-front; the worker processes them one at a time on its
    // own thread. Order is preserved, so sysb (content) precedes sysm (mask).
    let job_rx_list: Vec<_> = swf_inputs
        .iter()
        .zip(&job_scales)
        .map(|(input, &scale)| {
            tracing::info!(
                swf = %input.name,
                dims = format!("{}x{}", input.width as u32, input.height as u32),
                scale,
                "queued for render"
            );
            worker
                .send_job(input.path.clone(), scale)
                .with_context(|| format!("queueing render job for {}", input.name))
        })
        .collect::<Result<Vec<_>>>()?;

    let total_swf = swf_inputs.len();
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
    let mut writer = crate::pdf::PdfWriter::new(&out);
    let mut total_pages = 0u32;
    for res in pages {
        match res {
            Ok(page) => {
                total_pages += 1;
                // Dimensions are known from the worker — skip the JPEG header
                // parse in the PDF writer.
                writer.add_jpeg_with_dims(&page.jpeg, page.width, page.height)?;
            }
            Err(e) => {
                tracing::warn!(error = %e, "frame dropped");
            }
        }
    }
    writer.finish()?;

    // Shut down the worker (job_tx is dropped → worker loop ends). A join
    // error is only logged, not fatal — the PDF is already fully written.
    if let Err(e) = worker.join() {
        tracing::warn!(error = ?e, "render worker join error");
    }

    tracing::info!(pages = total_pages, "render PDF finished");
    Ok(())
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
