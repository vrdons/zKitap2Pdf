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
use crate::utils::find_real_size;
use crate::{executable, p, utils};

#[derive(Debug, Clone)]
pub struct HandleArgs {
    pub file: Files,
    pub scale: f64,
}

pub fn handle_exe(exporter: &Exporter, args: HandleArgs) -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let input_file = args.file.input.clone();
    let mut width = 566.0;
    let mut height = 807.0;
    let mut doc = Document::new();
    doc.set_title(args.file.filename);
    doc.set_author("Rust Developer");
    let (tx, rx): (Sender<ExporterEvents>, Receiver<ExporterEvents>) = channel();
    let mut swf_queue: VecDeque<NamedTempFile> = VecDeque::new();
    let mut jpeg_queue: VecDeque<NamedTempFile> = VecDeque::new();
    start_child(&input_file)?;

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
            ExporterEvents::FinishSWF => {
                println!("-- Finished Processing SWF");
                while let Some(next_file) = jpeg_queue.pop_front() {
                    let width = (width * args.scale).round();
                    let height = (height * args.scale).round();
                    let mut page = Page::new(width, height);
                    let pdf_image = Image::from_jpeg_file(next_file.path())?;
                    page.add_image("img", pdf_image);
                    page.draw_image("img", 0.0, 0.0, width, height)?;
                    doc.add_page(page);
                }
                if let Some(next_file) = swf_queue.pop_front() {
                    let file_path_for_exporter = next_file.path().to_path_buf();
                    println!("-- Started Processing: {:?}", file_path_for_exporter);
                    current_swf_file = Some(next_file);
                    start_exporter(exporter, &file_path_for_exporter, &temp_dir, |file| {
                        let _ = tx.send(ExporterEvents::Frame(file));
                    })?;
                    let _ = tx.send(ExporterEvents::FinishSWF);
                } else {
                    println!("-- No more swf to process");
                    if tx.send(ExporterEvents::FinishPDF).is_err() {
                        break;
                    }
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
                    let (w, h) = find_real_size(&decompressed)?;
                    println!("Real Size: {:?}", (w, h));
                    width = w;
                    height = h;
                    utils::patch_swf(decompressed, width, height)?
                };
                let mut patched_file = tempfile::NamedTempFile::new_in(temp_dir.path())?;
                patched_file.write_all(&patched)?;
                swf_queue.push_back(patched_file);
                if current_swf_file.is_none() {
                    if let Some(next_file) = swf_queue.pop_front() {
                        let file_path_for_exporter = next_file.path().to_path_buf();
                        println!("-- Started Processing: {:?}", file_path_for_exporter);
                        current_swf_file = Some(next_file);
                        start_exporter(exporter, &file_path_for_exporter, &temp_dir, |file| {
                            let _ = tx.send(ExporterEvents::Frame(file));
                        })?;
                        let _ = tx.send(ExporterEvents::FinishSWF);
                    }
                }
            }
            ExporterEvents::FoundProcess(file_path) => {
                println!("Found Process file: {:?}", file_path.path());
                let mut read = File::open(file_path.path())?;
                let decompressed = swf::decompress_swf(&mut read)?;
                if let Some(json) = p::check_process(&decompressed)? {
                    println!("{:?}", json);
                    let kkobj = p::get_kkobject(json)?;
                    println!("{:?}", kkobj);
                }
            }
        }
    }
    drop(tx);
    match t1.join() {
        Ok(result) => result?,
        Err(_) => return Err(anyhow::anyhow!("Watcher thread panicked")),
    }
    Ok(())
}

fn start_exporter(
    exporter: &Exporter,
    input: &PathBuf,
    temp_dir: &TempDir,
    mut on_frame: impl FnMut(NamedTempFile),
) -> Result<()> {
    exporter.capture_frames(input, |_, image| {
        let jpeg_buf = {
            let rgb_image = DynamicImage::ImageRgba8(image).to_rgb8();
            let mut jpeg_buf = Cursor::new(Vec::new());
            if rgb_image
                .write_to(&mut jpeg_buf, ImageFormat::Jpeg)
                .is_err()
            {
                eprintln!("Failed to encode JPEG");
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
        on_frame(temp_file);
    })?;

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
    let mut pending: HashMap<String, NamedTempFile> = HashMap::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(res) => {
                let event = res?;

                if !matches!(event.kind, notify::EventKind::Modify(_)) {
                    continue;
                }

                let Some(path) = event.paths.first() else {
                    continue;
                };

                let Some(name) = path.file_name() else {
                    continue;
                };

                let filename = name.to_string_lossy().to_string();

                let bytes = match fs::read(path) {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                if bytes.len() < 3 {
                    continue;
                }

                match &bytes[..3] {
                    b"FWS" | b"CWS" | b"ZWS" => {}
                    _ => continue,
                }

                last_activity = Instant::now();
                let entry = match pending.entry(filename) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => match NamedTempFile::new() {
                        Ok(f) => e.insert(f),
                        Err(err) => {
                            eprintln!("Failed to create temp file: {}", err);
                            continue;
                        }
                    },
                };

                fs::write(entry.path(), &bytes)?;
            }

            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let secs = if pending.is_empty() { 35 } else { 15 };
                if last_activity.elapsed() >= Duration::from_secs(secs) {
                    if pending.is_empty() {
                        println!("No SWF found. Watcher exiting silently.");
                        break;
                    }
                    let pending_len = pending.len();
                    for (name, tmpfile) in pending.into_iter() {
                        if name == "p.dll" && pending_len == 1 {
                            if sender.send(ExporterEvents::FoundProcess(tmpfile)).is_err() {
                                return Ok(());
                            }
                        } else if sender.send(ExporterEvents::FoundSWF(tmpfile)).is_err() {
                            return Ok(());
                        }
                    }

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
    FoundProcess(NamedTempFile),
    Frame(NamedTempFile),
    FinishSWF,
    FinishPDF,
}
