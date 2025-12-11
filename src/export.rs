use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Result;
use image::{DynamicImage, ImageFormat};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use oxidize_pdf::{Document, Image, Page};
use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Write};
use tempfile::NamedTempFile;

use crate::cli::Files;
use crate::exporter::Exporter;
use crate::{executable, utils};

#[derive(Debug, Clone)]
pub struct HandleArgs {
    pub file: Files,
    pub scale: f64,
}

pub fn handle_exe(exporter: &Exporter, args: HandleArgs) -> Result<()> {
    let input_file = args.file.input.clone();
    let width = (566.0 * args.scale).round();
    let height = (800.0 * args.scale).round();
    let mut doc = Document::new();
    doc.set_title(args.file.filename);
    doc.set_author("Rust Developer");
    let (tx, rx): (Sender<ExporterEvents>, Receiver<ExporterEvents>) = channel();
    let mut swf_queue: VecDeque<NamedTempFile> = VecDeque::new();
    let mut jpeg_queue: VecDeque<PathBuf> = VecDeque::new();
    start_child(&input_file)?;

    std::thread::sleep(std::time::Duration::from_millis(1000));
    println!("Child process finished, waiting for sys1.dll");
    let tx2 = tx.clone();
    let t1 = std::thread::spawn(move || watch_roaming(tx2));
    let mut current_swf_file: Option<NamedTempFile> = None;
    let mut frame_index = 0;
    let mut total_frames = 0;
    loop {
        let event = match rx.recv() {
            Ok(e) => e,
            Err(_) => {
                println!("Watcher thread işini bitirdi. Event dinleme sonlandırılıyor.");
                break;
            }
        };
        match event {
            ExporterEvents::Frame(temp_file) => {
                jpeg_queue.push_back(temp_file.path().to_path_buf());
                println!("Frame {} exported", frame_index);
                frame_index += 1;
            }
            ExporterEvents::FinishFrame => {
                if let Some(next_file) = swf_queue.pop_front() {
                    let file_path_for_exporter = next_file.path().to_path_buf();
                    println!("-- Started Processing: {:?}", file_path_for_exporter);
                    current_swf_file = Some(next_file);
                    start_exporter(
                        &exporter,
                        &file_path_for_exporter,
                        tx.clone(),
                        &mut total_frames,
                    )
                    .unwrap();
                } else {
                    println!("-- No more swf to process");
                    tx.send(ExporterEvents::FinishSWF).unwrap();
                }
            }
            ExporterEvents::FinishSWF => {
                println!("-- Finished Processing SWF");
                if frame_index < total_frames {
                    println!("-- Waiting for processing remaining jpegs");
                    std::thread::sleep(Duration::from_millis(10));
                }
                while frame_index < total_frames {
                    std::thread::sleep(Duration::from_millis(100));
                }
                while let Some(next_file) = jpeg_queue.pop_front() {
                    let mut page = Page::new(width, height);
                    let pdf_image = Image::from_jpeg_file(next_file.clone())?;
                    page.add_image("img", pdf_image);
                    page.draw_image("img", 0.0, 0.0, width, height)?;
                    doc.add_page(page);
                    fs::remove_file(next_file)?;
                }
                tx.send(ExporterEvents::FinishPDF).unwrap();
            }
            ExporterEvents::FinishPDF => {
                println!("-- Finished Processing PDF");
                let pdf_path = args.file.output.clone();
                println!("-- Exporting PDF to: {:?}", pdf_path);
                doc.save(&pdf_path)?;
            }
            ExporterEvents::FoundSWF(file_path) => {
                println!("Found SWF file: {:?}", file_path.path());
                swf_queue.push_back(file_path);
                if current_swf_file.is_none() {
                    if let Some(next_file) = swf_queue.pop_front() {
                        let file_path_for_exporter = next_file.path().to_path_buf();
                        println!("-- Started Processing: {:?}", file_path_for_exporter);
                        current_swf_file = Some(next_file);
                        start_exporter(
                            &exporter,
                            &file_path_for_exporter,
                            tx.clone(),
                            &mut total_frames,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    t1.join().expect("Error when joining thread")?;
    Ok(())
}

fn start_exporter(
    exporter: &Exporter,
    input: &PathBuf,
    tx: Sender<ExporterEvents>,
    total_frames: &mut u32,
) -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut file: File = File::open(input)?;
    let mut patched = {
        let decompressed = swf::decompress_swf(&mut file)?;
        *total_frames += decompressed.header.num_frames() as u32;
        utils::patch_swf(decompressed)?
    };
    exporter.capture_frames(&mut patched, |_, image, end| {
        let jpeg_buf = {
            let rgb_image = DynamicImage::ImageRgba8(image).to_rgb8();
            let mut jpeg_buf = Cursor::new(Vec::new());
            rgb_image.write_to(&mut jpeg_buf, ImageFormat::Jpeg).ok();
            jpeg_buf.into_inner()
        };
        let path = NamedTempFile::new_in(temp_dir.path()).unwrap();
        fs::write(path.path(), &jpeg_buf).ok();
        tx.send(ExporterEvents::Frame(path)).unwrap();
        if end {
            tx.send(ExporterEvents::FinishFrame).unwrap();
        }
    })?;

    Ok(())
}

fn start_child(input: &PathBuf) -> Result<()> {
    let mut exe = executable::execute_exe(&input)?;
    exe.wait()?;
    Ok(())
}

fn watch_roaming(sender: Sender<ExporterEvents>) -> Result<()> {
    let roaming_path = executable::get_roaming_path()?;
    println!("Watching roaming path: {:?}", roaming_path);
    let (tx, rx) = channel();

    let mut watcher: RecommendedWatcher = Watcher::new(tx, Config::default())?;
    watcher.watch(&roaming_path, RecursiveMode::Recursive)?;

    let mut last_activity = Instant::now();
    let mut last_processed: HashMap<String, Instant> = HashMap::new();
    const DEBOUNCE_DURATION: Duration = Duration::from_millis(1000);
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => {
                let event = event?;

                if !matches!(event.kind, notify::EventKind::Modify(_)) {
                    continue;
                }

                let Some(file_path) = event.paths.first() else {
                    continue;
                };
                let Some(filename_os) = file_path.file_name() else {
                    continue;
                };
                let filename = filename_os.to_string_lossy().to_string();

                const IGNORED_FILE: &str = "p.dll";
                if filename == IGNORED_FILE {
                    continue;
                }

                let Ok(bytes) = std::fs::read(file_path) else {
                    continue;
                };

                if bytes.len() < 3 {
                    continue;
                }
                let header = &bytes[..3];
                let is_swf = header == b"FWS" || header == b"CWS" || header == b"ZWS";
                if !is_swf {
                    continue;
                }

                last_activity = Instant::now();
                let now = Instant::now();
                if let Some(last_time) = last_processed.get(&filename) {
                    if now.duration_since(*last_time) < DEBOUNCE_DURATION {
                        continue;
                    }
                }
                last_processed.insert(filename.clone(), now);
                println!("Dosya yakalandı: {}", filename);
                let mut tempfile = NamedTempFile::new().expect("Failed to create temporary file");

                tempfile
                    .write_all(&bytes)
                    .expect("Failed to write to temporary file");
                sender.send(ExporterEvents::FoundSWF(tempfile)).ok();
            }

            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_activity.elapsed() >= Duration::from_secs(20) {
                    println!("Inactivity timeout: no events for 20 => exiting watcher");
                    break;
                }
            }

            Err(e) => {
                println!("watch error: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
pub enum ExporterEvents {
    FoundSWF(NamedTempFile),
    Frame(NamedTempFile),
    FinishFrame,
    FinishSWF,
    FinishPDF,
}
