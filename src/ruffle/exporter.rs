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
    sync::Arc,
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

    pub fn capture_frames<F>(&self, path: &Path, mut on_frame: F) -> Result<()>
    where
        F: FnMut(u16, RgbaImage),
    {
        let movie = movie_from_path(path, None)
            .map_err(|e| anyhow!("loading movie {}: {e}", path.display()))?;
        let total_frames = movie.num_frames();

        let width = ((movie.width().to_pixels() * self.scale).round() as u32).max(1);
        let height = ((movie.height().to_pixels() * self.scale).round() as u32).max(1);
        tracing::debug!(width, height, "render dimensions");

        let target = TextureTarget::new(&self.descriptors.device, (width, height))
            .map_err(|e| anyhow!("creating render texture target: {e}"))?;

        let player = PlayerBuilder::new()
            .with_renderer(
                WgpuRenderBackend::new(self.descriptors.clone(), target)
                    .map_err(|e| anyhow!("building wgpu render backend: {e}"))?,
            )
            .with_movie(movie)
            .with_viewport_dimensions(width, height, self.scale)
            .build();

        tracing::debug!(total_frames, "capturing frames");

        for i in 0..total_frames {
            let capture_attempt = {
                let mut locked_player =
                    player.lock().map_err(|e| anyhow!("mutex poisoned: {e}"))?;
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
                    tracing::info!(frame = i, "captured");
                    on_frame(i, img);
                }
                Ok(Ok(None)) => tracing::warn!(frame = i, "captured an empty image"),
                Ok(Err(e)) => {
                    return Err(anyhow!("Render/downcast error on frame {i}: {e:?}"));
                }
                Err(e) => return Err(anyhow!("Panicked on frame {i}: {e:?}")),
            }
        }

        Ok(())
    }
}
