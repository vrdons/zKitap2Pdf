use anyhow::{Result, anyhow};
use image::RgbaImage;
use ruffle_core::{PlayerBuilder, limits::ExecutionLimit, tag_utils::movie_from_path};
use ruffle_render_wgpu::{
    backend::{WgpuRenderBackend, request_adapter_and_device},
    clap::GraphicsBackend,
    descriptors::Descriptors,
    target::TextureTarget,
    wgpu,
};
use std::{
    any::Any,
    io::Cursor,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

pub struct ExporterOpt {
    pub graphics: GraphicsBackend,
    pub scale: f64,
    /// Approximate memory budget (MiB) for in-flight rendered frames.
    /// `0` = unbounded (default channel sizes). Bounds the capture/JPEG
    /// queues so low-memory machines don't pile up big RGBA/JPEG buffers.
    pub max_mem: usize,
}

pub struct Exporter {
    descriptors: Arc<Descriptors>,
    scale: f64,
    max_mem: usize,
}

/// One ready-to-embed page frame, already JPEG-encoded by the render worker so
/// the consumer never has to touch raw pixel buffers or spill to disk.
pub struct RenderedPage {
    /// Zero-based frame index within the source SWF.
    // Kept for diagnostics (which source page failed to encode); not consumed
    // by the PDF path, which relies on channel ordering instead.
    #[allow(dead_code)]
    pub frame: u16,
    /// JPEG-encoded image bytes (DCT stream), directly embeddable by oxidize-pdf.
    pub jpeg: Vec<u8>,
    /// Exact pixel dimensions of the rendered frame (pre-encoded by the
    /// worker) — avoids parsing the JPEG header in the PDF writer.
    pub width: u32,
    pub height: u32,
}

/// A single job for the persistent render worker.
pub struct RenderJob {
    pub path: PathBuf,
    /// Per-job channel — the worker sends JPEG-encoded pages (or errors) here.
    pub tx: mpsc::SyncSender<Result<RenderedPage>>,
}

/// Handle returned by [`Exporter::spawn_worker`].
pub struct Worker {
    handle: thread::JoinHandle<()>,
    job_tx: mpsc::SyncSender<RenderJob>,
    max_mem: usize,
}

impl Worker {
    /// Queue a SWF for rendering. Returns the per-job receiver that yields
    /// JPEG-encoded pages in order.  Blocks only if the worker's queue is full.
    pub fn send_job(&self, path: PathBuf) -> Result<mpsc::Receiver<Result<RenderedPage>>> {
        // A bounded per-job channel provides mild back-pressure so a book with
        // hundreds of pages doesn't queue every encoded JPEG in RAM before the
        // PDF writer drains it. With `--max-mem` the queue shrinks further to
        // keep in-flight JPEG bytes low on constrained machines.
        let queue_len = if self.max_mem > 0 { 2 } else { 4 };
        let (tx, rx) = mpsc::sync_channel(queue_len);
        self.job_tx
            .send(RenderJob { path, tx })
            .map_err(|_| anyhow!("render worker has died"))?;
        Ok(rx)
    }

    /// Shut down the worker and wait for it to finish.
    pub fn join(self) -> thread::Result<()> {
        drop(self.job_tx); // signal EOF
        self.handle.join()
    }
}

impl Exporter {
    pub fn new(opt: &ExporterOpt) -> Result<Self> {
        let backend = opt.graphics.into();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: backend,
            ..Default::default()
        });

        let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
            backend,
            &instance,
            None,
            wgpu::PowerPreference::HighPerformance,
        ))
        .map_err(|e| anyhow!("requesting wgpu adapter/device: {e}"))?;

        Ok(Self {
            descriptors: Arc::new(Descriptors::new(instance, adapter, device, queue)),
            scale: opt.scale,
            max_mem: opt.max_mem,
        })
    }

    /// Spawn a **single** persistent render worker thread.  All SWF rendering
    /// jobs are sent through the returned [`Worker`] so the GLES context is
    /// never shared across threads.
    pub fn spawn_worker(&self) -> Worker {
        // The job queue holds *paths* (tiny), so it can stay generous; the
        // per-job page queues are where big buffers accumulate.
        let (job_tx, job_rx) = mpsc::sync_channel::<RenderJob>(8);
        let descriptors = self.descriptors.clone();
        let scale = self.scale;
        let max_mem = self.max_mem;

        let handle = thread::spawn(move || {
            for job in job_rx {
                render_frames(descriptors.clone(), scale, max_mem, &job.path, job.tx);
            }
        });

        Worker {
            handle,
            job_tx,
            max_mem: self.max_mem,
        }
    }
}

/// Run the full render loop for a single SWF, sending each captured frame
/// through `tx`.  Called by the worker thread.
fn render_frames(
    descriptors: Arc<Descriptors>,
    scale: f64,
    max_mem: usize,
    path: &Path,
    tx: mpsc::SyncSender<Result<RenderedPage>>,
) {
    // Clone the sender so the encoder thread can move its own copy while the
    // render loop keeps using `tx` for error reporting.
    let send_err = {
        let tx = tx.clone();
        move |frame: u16, msg: &str| {
            let _ = tx.send(Err(anyhow!("frame {frame}: {msg}")));
        }
    };

    let movie = match movie_from_path(path, None) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(Err(anyhow!("loading movie {}: {e}", path.display())));
            return;
        }
    };
    let total_frames = movie.num_frames();

    let width = ((movie.width().to_pixels() * scale).round() as u32).max(1);
    let height = ((movie.height().to_pixels() * scale).round() as u32).max(1);

    let target = match TextureTarget::new(&descriptors.device, (width, height)) {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(Err(anyhow!("creating render texture target: {e}")));
            return;
        }
    };

    let renderer = match WgpuRenderBackend::new(descriptors.clone(), target) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(anyhow!("building wgpu render backend: {e}")));
            return;
        }
    };

    let player = PlayerBuilder::new()
        .with_renderer(renderer)
        .with_movie(movie)
        .with_viewport_dimensions(width, height, scale)
        .build();

    tracing::info!(total_frames, %width, %height, "capturing frames");

    // Pipeline: the render loop pushes raw RGBA captures into a bounded queue,
    // a dedicated encoder thread converts them to JPEG and forwards them in
    // order. GPU render and CPU encode overlap, so the second core stays busy
    // instead of idling while the worker serialises encode after render.
    // With `--max-mem` the capture queue shrinks to 1, bounding how many
    // uncompressed RGBA buffers can be in flight (~W×H×4 bytes each).
    let cap_queue = if max_mem > 0 { 1 } else { 2 };
    let (cap_tx, cap_rx) = mpsc::sync_channel::<Result<(u16, RgbaImage)>>(cap_queue);

    let encode_handle = thread::spawn(move || {
        for item in cap_rx {
            match item {
                Ok((frame, rgba)) => {
                    let (w, h) = (rgba.width(), rgba.height());
                    match encode_jpeg(&rgba) {
                        Ok(jpeg) => {
                            if tx
                                .send(Ok(RenderedPage {
                                    frame,
                                    jpeg,
                                    width: w,
                                    height: h,
                                }))
                                .is_err()
                            {
                                break; // consumer dropped
                            }
                            if (frame + 1) % 25 == 0 {
                                tracing::info!(
                                    page = frame + 1,
                                    total = total_frames,
                                    "captured"
                                );
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(anyhow!("frame {frame}: jpeg encode: {e}")));
                        }
                    }
                }
                Err(e) => {
                    if tx.send(Err(e)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    for i in 0..total_frames {
        tracing::trace!(frame = i + 1, total = total_frames, "running frame");
        let capture_attempt = {
            let mut locked_player = match player.lock() {
                Ok(pl) => pl,
                Err(e) => {
                    send_err(i, &format!("mutex poisoned: {e}"));
                    break;
                }
            };
            locked_player.preload(&mut ExecutionLimit::none());
            locked_player.run_frame();
            locked_player.render();

            catch_unwind(AssertUnwindSafe(|| {
                let renderer = <dyn Any>::downcast_mut::<WgpuRenderBackend<TextureTarget>>(
                    locked_player.renderer_mut(),
                )
                .ok_or_else(|| anyhow!("Renderer type mismatch"))?;
                Ok::<Option<RgbaImage>, anyhow::Error>(renderer.capture_frame())
            }))
        };

        match capture_attempt {
            Ok(Ok(Some(rgba))) => {
                // Hand off to the encoder thread. A full queue means the
                // encoder is still busy — back-pressure is fine, it just
                // synchronises the pipeline briefly.
                if cap_tx.send(Ok((i, rgba))).is_err() {
                    break; // encoder died
                }
            }
            Ok(Ok(None)) => {
                send_err(i, "empty capture");
            }
            Ok(Err(e)) => {
                send_err(i, &format!("render/downcast: {e}"));
            }
            Err(e) => {
                send_err(i, &format!("panic: {e:?}"));
            }
        }
    }

    // Signal EOF to the encoder and wait for it to drain remaining frames.
    drop(cap_tx);
    let _ = encode_handle.join();
}

/// In-memory JPEG encoding of a captured RGBA frame.
///
/// Runs on a dedicated encoder thread, keeping the (relatively expensive)
/// pixel → DCT conversion off both the GPU render loop and the PDF-writer
/// thread.
fn encode_jpeg(rgba: &RgbaImage) -> Result<Vec<u8>> {
    let rgb = image::DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
    let mut buf = Vec::with_capacity((rgb.width() as usize) * (rgb.height() as usize) / 4);
    rgb.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .map_err(|e| anyhow!("jpeg encode: {e}"))?;
    Ok(buf)
}
