use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Result;
use image::{DynamicImage, ImageFormat};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use oxidize_pdf::{Document, Image, Page};
use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Write};
use tempfile::{NamedTempFile, TempDir};

use crate::cli::Files;
use crate::exporter::Exporter;
use crate::{executable, utils};

#[derive(Debug, Clone)]
pub struct HandleArgs {
    pub file: Files,
    pub scale: f64,
}

pub fn handle_exe(exporter: &Exporter, args: HandleArgs) -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let input_file = args.file.input.clone();
    let width = (566.0 * args.scale).round();
    let height = (800.0 * args.scale).round();
    let mut doc = Document::new();
    doc.set_title(args.file.filename);
    doc.set_author("Rust Developer");
    let (tx, rx): (Sender<ExporterEvents>, Receiver<ExporterEvents>) = channel();
    let mut swf_queue: VecDeque<NamedTempFile> = VecDeque::new();
    let mut jpeg_queue: VecDeque<NamedTempFile> = VecDeque::new();
    start_child(&input_file)?;

    std::thread::sleep(std::time::Duration::from_millis(1000));
    println!("Child process finished, waiting for sys1.dll");
    let tx2 = tx.clone();
    let t1 = std::thread::spawn(move || watch_roaming(tx2));
    let mut current_swf_file: Option<NamedTempFile> = None;
    let mut frame_index = 0;
    loop {
        let event = match rx.recv() {
            Ok(e) => e,
            Err(_) => {
                println!("Watcher thread finished. Stopping event listener.");
                break;
            }
        };
        match event {
            ExporterEvents::Frame(temp_file) => {
                jpeg_queue.push_back(temp_file);
                println!("Frame {} exported", frame_index);
                frame_index += 1;
            }
            ExporterEvents::FinishFrame => {
                if let Some(next_file) = swf_queue.pop_front() {
                    let file_path_for_exporter = next_file.path().to_path_buf();
                    println!("-- Started Processing: {:?}", file_path_for_exporter);
                    current_swf_file = Some(next_file);
                    start_exporter(exporter, &file_path_for_exporter, tx.clone(), &temp_dir)?;
                } else {
                    println!("-- No more swf to process");
                    if tx.send(ExporterEvents::FinishSWF).is_err() {
                        break;
                    }
                }
            }
            ExporterEvents::FinishSWF => {
                println!("-- Finished Processing SWF");
                while let Some(next_file) = jpeg_queue.pop_front() {
                    let mut page = Page::new(width, height);
                    let pdf_image = Image::from_jpeg_file(next_file.path())?;
                    page.add_image("img", pdf_image);
                    page.draw_image("img", 0.0, 0.0, width, height)?;
                    doc.add_page(page);
                }
                if tx.send(ExporterEvents::FinishPDF).is_err() {
                    break;
                }
            }
            ExporterEvents::FinishPDF => {
                println!("-- Finished Processing PDF");
                let pdf_path = args.file.output.clone();
                println!("-- Exporting PDF to: {:?}", pdf_path);
                doc.save(&pdf_path)?;
                break;
            }
            ExporterEvents::FoundSWF(file_path) => {
                println!("Found SWF file: {:?}", file_path.path());
                let mut read = File::open(file_path.path())?;
                let patched = {
                    let decompressed = swf::decompress_swf(&mut read)?;
                    utils::patch_swf(decompressed)?
                };
                fs::write(file_path.path(), &patched)?;
                swf_queue.push_back(file_path);
                if current_swf_file.is_none() {
                    if let Some(next_file) = swf_queue.pop_front() {
                        let file_path_for_exporter = next_file.path().to_path_buf();
                        println!("-- Started Processing: {:?}", file_path_for_exporter);
                        current_swf_file = Some(next_file);
                        start_exporter(exporter, &file_path_for_exporter, tx.clone(), &temp_dir)?;
                    }
                }
            }
        }
    }
    match t1.join() {
        Ok(result) => result?,
        Err(_) => return Err(anyhow::anyhow!("Watcher thread panicked")),
    }
    Ok(())
}

fn start_exporter(
    exporter: &Exporter,
    input: &PathBuf,
    tx: Sender<ExporterEvents>,
    temp_dir: &TempDir,
) -> Result<()> {
    exporter.capture_frames(input, |_, image| {
        let jpeg_buf = {
            let rgb_image = DynamicImage::ImageRgba8(image).to_rgb8();
            let mut jpeg_buf = Cursor::new(Vec::new());
            if let Err(e) = rgb_image.write_to(&mut jpeg_buf, ImageFormat::Jpeg) {
                eprintln!("Failed to encode JPEG: {}", e);
            }
            jpeg_buf.into_inner()
        };
        let mut temp_file = match NamedTempFile::new_in(temp_dir.path()) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to create temp file: {}", e);
                return;
            }
        };
        if let Err(e) = temp_file.write_all(&jpeg_buf) {
            eprintln!("Failed to write temp file: {}", e);
        }
        let _ = tx.send(ExporterEvents::Frame(temp_file));
    })?;

    let _ = tx.send(ExporterEvents::FinishFrame);
    Ok(())
}

fn start_child(input: &Path) -> Result<()> {
    let mut exe = executable::execute_exe(input)?;
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
    const DEBOUNCE_DURATION: Duration = Duration::from_millis(2000);
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
                println!("File captured: {}", filename);
                let mut tempfile = match NamedTempFile::new() {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("Failed to create temporary file: {}", e);
                        continue;
                    }
                };
                if let Err(e) = tempfile.write_all(&bytes) {
                    eprintln!("Failed to write to temporary file: {}", e);
                    continue;
                }
                let _ = sender.send(ExporterEvents::FoundSWF(tempfile));
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
