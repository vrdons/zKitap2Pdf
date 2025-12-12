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
    path::PathBuf,
    sync::Arc,
};

#[derive(Copy, Clone)]
pub struct SizeOpt {
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: opt.graphics.into(),
            ..Default::default()
        });

        let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
            opt.graphics.into(),
            &instance,
            None,
            wgpu::PowerPreference::HighPerformance,
        ))
        .map_err(|e| anyhow!(e.to_string()))?;

        let descriptors = Arc::new(Descriptors::new(instance, adapter, device, queue));

        Ok(Self {
            descriptors,
            scale: opt.scale,
        })
    }

    pub fn capture_frames<F>(&self, path: &PathBuf, mut on_frame: F) -> Result<()>
    where
        F: FnMut(u16, RgbaImage),
    {
        let movie = movie_from_path(path, None).map_err(|e| anyhow!(e.to_string()))?;
        let total_frames = movie.num_frames();

        let width = movie.width().to_pixels();
        let width = (width * self.scale).round() as u32;

        let height = movie.height().to_pixels();
        let height = (height * self.scale).round() as u32;
        println!("Width: {} Height: {}", width, height);

        let target = TextureTarget::new(&self.descriptors.device, (width, height))
            .map_err(|e| anyhow!(e.to_string()))?;

        let player = PlayerBuilder::new()
            .with_renderer(
                WgpuRenderBackend::new(self.descriptors.clone(), target)
                    .map_err(|e| anyhow!(e.to_string()))?,
            )
            .with_movie(movie)
            .with_viewport_dimensions(width, height, self.scale)
            .build();

        println!("Total Frames: {}", total_frames);

        for i in 0..total_frames {
            let capture_attempt: Result<Result<Option<RgbaImage>>, Box<dyn Any + Send>> = {
                let mut locked_player = player
                    .lock()
                    .map_err(|e| anyhow!("Mutex poisoned: {}", e))?;

                locked_player.preload(&mut ExecutionLimit::none());
                locked_player.run_frame();
                locked_player.render();

                catch_unwind(AssertUnwindSafe(|| {
                    let renderer = <dyn Any>::downcast_mut::<WgpuRenderBackend<TextureTarget>>(
                        locked_player.renderer_mut(),
                    )
                    .ok_or_else(|| anyhow!("Renderer type mismatch"))?;

                    let frame: Option<RgbaImage> = renderer.capture_frame();
                    Ok(frame)
                }))
            };

            match capture_attempt {
                Ok(Ok(Some(img))) => {
                    println!("Frame {} captured.", i);
                    on_frame(i, img);
                }
                Ok(Ok(None)) => {
                    eprintln!("WARN: Frame {} captured an empty image.", i);
                }
                Ok(Err(e)) => {
                    return Err(anyhow!("render/downcast error on frame {}: {:?}", i, e));
                }
                Err(e) => {
                    eprintln!("Paniced on frame {}: {:?}", i, e);
                }
            }
        }

        Ok(())
    }
}
