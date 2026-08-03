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
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
};

pub struct ExporterOpt {
    pub graphics: GraphicsBackend,
    pub scale: f64,
}

pub struct Exporter {
    descriptors: Arc<Descriptors>,
    scale: f64,
}

/// A single job for the persistent render worker.
pub struct RenderJob {
    pub path: PathBuf,
    /// Per-job channel — the worker sends captured frames (or errors) here.
    pub tx: mpsc::SyncSender<Result<(u16, RgbaImage)>>,
}

/// Handle returned by [`Exporter::spawn_worker`].
pub struct Worker {
    handle: thread::JoinHandle<()>,
    job_tx: mpsc::SyncSender<RenderJob>,
}

impl Worker {
    /// Queue a SWF for rendering. Returns the per-job receiver that yields
    /// captured frames in order.  Blocks only if the worker's queue is full.
    pub fn send_job(&self, path: PathBuf) -> Result<mpsc::Receiver<Result<(u16, RgbaImage)>>> {
        let (tx, rx) = mpsc::sync_channel(16);
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
        })
    }

    /// Spawn a **single** persistent render worker thread.  All SWF rendering
    /// jobs are sent through the returned [`Worker`] so the GLES context is
    /// never shared across threads.
    pub fn spawn_worker(&self) -> Worker {
        let (job_tx, job_rx) = mpsc::sync_channel::<RenderJob>(8);
        let descriptors = self.descriptors.clone();
        let scale = self.scale;

        let handle = thread::spawn(move || {
            for job in job_rx {
                render_frames(descriptors.clone(), scale, &job.path, job.tx);
            }
        });

        Worker { handle, job_tx }
    }
}

/// Run the full render loop for a single SWF, sending each captured frame
/// through `tx`.  Called by the worker thread.
fn render_frames(
    descriptors: Arc<Descriptors>,
    scale: f64,
    path: &Path,
    tx: mpsc::SyncSender<Result<(u16, RgbaImage)>>,
) {
    let send_err = |frame: u16, msg: &str| {
        let _ = tx.send(Err(anyhow!("frame {frame}: {msg}")));
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

    tracing::info!(total_frames, "capturing frames");

    let mut exec_limit = ExecutionLimit::new();
    exec_limit.max_actions_per_frame = 200_000;
    exec_limit.max_recursion_depth = 64;

    for i in 0..total_frames {
        tracing::info!(frame = i + 1, total = total_frames, "running frame");
        let capture_attempt = {
            let mut locked_player = match player.lock() {
                Ok(pl) => pl,
                Err(e) => {
                    send_err(i, &format!("mutex poisoned: {e}"));
                    return;
                }
            };
            locked_player.preload(&mut exec_limit.clone());
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
            Ok(Ok(Some(img))) => {
                tracing::info!(page = i + 1, total = total_frames, "captured");
                if tx.send(Ok((i, img))).is_err() {
                    break; // consumer dropped
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
}
