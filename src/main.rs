use std::{
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    cli::Args,
    executable::{execute_exe, get_roaming_path, setup_environment},
    utils::patch_swf,
};

use clap::Parser;
use image::{DynamicImage, ImageFormat, imageops};
use oxidize_pdf::{Document, Image, Page};

pub mod cli;
pub mod executable;
pub mod exporter;
pub mod utils;

fn main() -> anyhow::Result<()> {
    let arg = Args::parse();
    let (files, scale) = arg.validate()?;
    let exporter = exporter::Exporter::new(&exporter::ExporterOpt {
        graphics: arg.graphics,
        scale,
    })?;
    setup_environment()?;
    let swf_files: utils::FileBuffer = Arc::new(Mutex::new(Vec::new()));
    let swf_files_thread = swf_files.clone();

    let _t1 = std::thread::spawn(move || {
        let roaming = get_roaming_path().unwrap();
        utils::watch_file(&roaming, swf_files_thread)
            .unwrap_or_else(|e| println!("watch rc: {}", e));
    });
    for file in files {
        println!("Processing : {:?}", file.input);
        let input = file.input;
        let output = file.output;
        let mut doc = Document::new();
        doc.set_title(file.filename);
        doc.set_author("Rust Developer");

        execute_exe(&input)?.wait()?;

        println!("Process finished");
        println!("Waiting for files to be copied...");
        let deadline = std::time::Instant::now() + Duration::from_secs(25);

        loop {
            {
                let lock = swf_files.lock().unwrap();

                if lock
                    .iter()
                    .any(|(name, _)| name.starts_with("sys") && name.ends_with(".dll"))
                {
                    println!(
                        "sys DLL detected, waiting 4 seconds more to copy full files (sys1,sys2 etc.)."
                    );
                    std::thread::sleep(Duration::from_secs(4));
                    break;
                }
            }

            if std::time::Instant::now() > deadline {
                println!("Timeout reached, continue anyway.");
                break;
            }

            std::thread::sleep(Duration::from_millis(200));
        }

        println!("End!");

        let mut index = 0;
        let lock = swf_files.lock().unwrap();

        if lock.is_empty() {
            anyhow::bail!("No SWF files found");
        }
        let mut sorted: Vec<_> = lock.clone();

        utils::sort_files(&mut sorted);
        println!("Sorted files");
        for (name, data) in sorted {
            println!("Processing : {:?}", name);
            let mut lock = swf_files.lock().unwrap();
            lock.retain(|(n, _)| n != &name);
            let mut patched = patch_swf(&data)?;
            let frames = exporter.capture_frames(&mut patched)?;
            for image in frames.iter() {
                let width = image.width() as f64;
                let height = image.height() as f64;
                let mut page = Page::new(width, height);
                let jpeg_buf = {
                    let rgba_clone = image.clone();

                    let rgb_image = DynamicImage::ImageRgba8(rgba_clone).to_rgb8();

                    let sharpened = imageops::unsharpen(&rgb_image, 0.8, 0);

                    let mut buf = Vec::with_capacity(256 * 1024);
                    {
                        let mut cursor = Cursor::new(&mut buf);
                        sharpened.write_to(&mut cursor, ImageFormat::Jpeg)?;
                    }

                    buf
                };
                let pdf_image = Image::from_jpeg_data(jpeg_buf)?;

                page.add_image("img", pdf_image);
                page.draw_image("img", 0.0, 0.0, width, height)?;
                doc.add_page(page);
                println!("Added frame to pdf: {}", index);
                index += 1;
            }
        }
        println!("Finished frames, saving pdf file");
        doc.save(output.to_string_lossy().to_string())?;
        let mut lock = swf_files.lock().unwrap();
        lock.clear();
        std::thread::sleep(Duration::from_millis(1000));
    }
    Ok(())
}
