use std::any::Any;
use std::fs::File;
use std::panic::catch_unwind;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use image::RgbaImage;
use ruffle_core::Player;
use ruffle_core::PlayerBuilder;
use ruffle_core::limits::ExecutionLimit;
use ruffle_core::tag_utils::SwfMovie;
use ruffle_core::tag_utils::movie_from_path;
use ruffle_render_wgpu::backend::WgpuRenderBackend;
use ruffle_render_wgpu::clap::GraphicsBackend;
use ruffle_render_wgpu::descriptors::Descriptors;

use anyhow::Result;
use anyhow::anyhow;
use ruffle_render_wgpu::backend::request_adapter_and_device;
use ruffle_render_wgpu::target::TextureTarget;
use ruffle_render_wgpu::wgpu;

#[derive(Copy, Clone)]
pub struct SizeOpt {
    pub scale: f64,

    pub width: u32,

    pub height: u32,
}

pub struct Opt {
    pub graphics: GraphicsBackend,
    pub size: SizeOpt,
}

pub struct Exporter {
    descriptors: Arc<Descriptors>,
    size: SizeOpt,
}

impl Exporter {
    pub fn new(opt: &Opt) -> Result<Self> {
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
            size: opt.size,
        })
    }

    pub fn start_exporting_movie(&self, file: &mut Vec<u8>) -> Result<MovieExport> {
        let movie = SwfMovie::from_data(file, "".to_string(), None)?;
        let width = self.size.width as f64;
        let width = (width * self.size.scale).round() as u32;

        let height = self.size.height as f64;
        let height = (height * self.size.scale).round() as u32;

        let target = TextureTarget::new(&self.descriptors.device, (width, height))
            .map_err(|e| anyhow!(e.to_string()))?;
        let player = PlayerBuilder::new()
            .with_renderer(
                WgpuRenderBackend::new(self.descriptors.clone(), target)
                    .map_err(|e| anyhow!(e.to_string()))?,
            )
            .with_movie(movie)
            .with_viewport_dimensions(width, height, self.size.scale)
            .build();

        Ok(MovieExport { player })
    }
}

pub struct MovieExport {
    player: Arc<Mutex<Player>>,
}

impl MovieExport {
    pub fn total_frames(&self) -> u32 {
        self.player.header_frames() as u32
    }

    pub fn run_frame(&self) {
        self.player
            .lock()
            .unwrap()
            .preload(&mut ExecutionLimit::none());

        self.player.lock().unwrap().run_frame();
    }

    pub fn capture_frame(&self) -> Result<RgbaImage> {
        let image = || {
            self.player.lock().unwrap().render();
            self.player.capture_frame()
        };
        match catch_unwind(image) {
            Ok(Some(image)) => Ok(image),
            Ok(None) => Err(anyhow!("No frame captured")),
            Err(e) => Err(anyhow!("{e:?}")),
        }
    }
}

pub trait PlayerExporterExt {
    fn capture_frame(&self) -> Option<image::RgbaImage>;

    fn header_frames(&self) -> u16;

    fn force_root_clip_play(&self);
}

impl PlayerExporterExt for Arc<Mutex<Player>> {
    fn capture_frame(&self) -> Option<image::RgbaImage> {
        let mut player = self.lock().unwrap();
        let renderer =
            <dyn Any>::downcast_mut::<WgpuRenderBackend<TextureTarget>>(player.renderer_mut())
                .unwrap();
        renderer.capture_frame()
    }

    fn header_frames(&self) -> u16 {
        self.lock()
            .unwrap()
            .mutate_with_update_context(|ctx| ctx.root_swf.num_frames())
    }

    fn force_root_clip_play(&self) {
        let mut player = self.lock().unwrap();

        // Check and resume if suspended
        if !player.is_playing() {
            player.set_is_playing(true);
        }

        // Also resume the root MovieClip if stopped
        player.mutate_with_update_context(|ctx| {
            if let Some(root_clip) = ctx.stage.root_clip()
                && let Some(movie_clip) = root_clip.as_movie_clip()
                && !movie_clip.playing()
            {
                movie_clip.play();
            }
        });
    }
}
