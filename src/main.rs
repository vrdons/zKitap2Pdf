use std::{
    fs::File,
    io::Cursor,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    cli::Args,
    executable::{execute_exe, get_roaming_path, setup_environment},
    utils::{clear_dir, find_files, take_screenshot},
};

use clap::Parser;
use image::{DynamicImage, ImageFormat};
use oxidize_pdf::{Document, Image, Page};
use swf::{Header, Rectangle, Twips, write::write_swf_raw_tags};

pub mod cli;
pub mod executable;
pub mod exporter;
pub mod paths;
pub mod utils;

fn main() -> anyhow::Result<()> {
    let arg = Args::parse();
    let (files, scale) = arg.validate()?;
    let temp_dir = Path::new(paths::TEMP_DIR);
    let exporter = exporter::Exporter::new(&exporter::Opt {
        graphics: arg.graphics,
        scale,
    })?;
    setup_environment()?;

    for file in files {
        println!("Processing : {:?}", file.input);
        let input = file.input;
        let output = file.output;
        let mut doc = Document::new();
        doc.set_title(file.filename);
        doc.set_author("Rust Developer");

        clear_dir(&temp_dir.to_path_buf())?;

        let stop_watch = Arc::new(AtomicBool::new(false));
        let roaming = get_roaming_path()?;

        let rc = roaming.clone();
        let tmp = temp_dir.to_path_buf().clone();
        let stp = stop_watch.clone();

        let _t1 = std::thread::spawn(move || {
            utils::watch_and_copy_swf(&rc, &tmp, stp)
                .unwrap_or_else(|e| println!("watch rc: {}", e));
        });

        execute_exe(&input)?.wait()?;

        //Sleeping for 5 seconds to allow the watcher to copy the files
        // TODO: waiting for file size
        std::thread::sleep(Duration::from_millis(15000));
        stop_watch.store(true, Ordering::Relaxed);

        let dlls = find_files(temp_dir, "dll")?;
        let mut i = 0;
        for dll in dlls {
            let file = File::open(dll)?;

            let dec = swf::decompress_swf(file)?;
            let header = Header {
                version: dec.header.version(),
                compression: dec.header.compression(),
                stage_size: Rectangle {
                    x_min: Twips::ZERO,
                    x_max: Twips::from_pixels(566.0),
                    y_min: Twips::ZERO,
                    y_max: Twips::from_pixels(807.0),
                },
                frame_rate: dec.header.frame_rate(),
                num_frames: dec.header.num_frames(),
            };
            let mut out = Cursor::new(Vec::<u8>::new());
            write_swf_raw_tags(&header, &dec.data, &mut out)?;

            let frames = take_screenshot(&exporter, &mut out.into_inner())?;
            for image in frames.iter() {
                let width = image.width() as f64;
                let height = image.height() as f64;
                let mut page = Page::new(width, height);
                let rgb_image = DynamicImage::ImageRgba8(image.clone()).to_rgb8();

                let mut jpeg_buf = Cursor::new(Vec::new());
                rgb_image.write_to(&mut jpeg_buf, ImageFormat::Jpeg)?;

                let pdf_image = Image::from_jpeg_data(jpeg_buf.into_inner())?;

                page.add_image("img", pdf_image);
                page.draw_image("img", 0.0, 0.0, width, height)?;
                doc.add_page(page);
                println!("Added frame to pdf: {}", i);
                i += 1;
            }
        }
        println!("Finished frames, saving pdf file");
        doc.save(output.to_string_lossy().to_string())?;
        std::thread::sleep(Duration::from_millis(1000));
    }
    Ok(())
}
