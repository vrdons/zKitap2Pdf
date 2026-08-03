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
    path::Path,
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

    /// Spawn WGPU rendering in a background thread; receive frames through a
    /// bounded channel (capacity 16 ≈ max ~224 MB) so the render thread can
    /// keep working ahead even when the main thread is busy encoding/PDF-ing
    /// earlier SWFs.
    pub fn capture_frames_threaded(
        &self,
        path: &Path,
        thread_id: u32,
    ) -> Result<(thread::JoinHandle<()>, mpsc::Receiver<Result<(u16, RgbaImage)>>)> {
        let (tx, rx) = mpsc::sync_channel::<Result<(u16, RgbaImage)>>(16);
        let descriptors = self.descriptors.clone();
        let scale = self.scale;
        let path_buf = path.to_path_buf();

        let handle = thread::spawn(move || {
            render_frames(descriptors, scale, &path_buf, tx, thread_id);
        });

        Ok((handle, rx))
    }
}

/// Run the full render loop in a background thread, sending each captured
/// frame through `tx`.  Errors are also sent through the channel so the
/// consumer sees them (the thread itself does not abort on a single failure).
fn render_frames(
    descriptors: Arc<Descriptors>,
    scale: f64,
    path: &Path,
    tx: mpsc::SyncSender<Result<(u16, RgbaImage)>>,
    thread_id: u32,
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

    let renderer = match WgpuRenderBackend::new(descriptors, target) {
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

    tracing::debug!(total_frames, "capturing frames (threaded)");

    for i in 0..total_frames {
        let capture_attempt = {
            let mut locked_player = match player.lock() {
                Ok(pl) => pl,
                Err(e) => {
                    send_err(i, &format!("mutex poisoned: {e}"));
                    return;
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
            Ok(Ok(Some(img))) => {
                tracing::info!(thread = thread_id, page = i + 1, "captured");
                if tx.send(Ok((i, img))).is_err() {
                    break; // consumer dropped
                }
            }
            Ok(Ok(None)) => {
                tracing::warn!(thread = thread_id, page = i + 1, "empty capture");
                send_err(i, "empty capture");
            }
            Ok(Err(e)) => {
                tracing::warn!(thread = thread_id, page = i + 1, error = %e, "render/downcast error");
                send_err(i, &format!("render/downcast: {e:?}"));
            }
            Err(e) => {
                tracing::warn!(thread = thread_id, page = i + 1, error = ?e, "panic");
                send_err(i, &format!("panic: {e:?}"));
            }
        }
    }
}
