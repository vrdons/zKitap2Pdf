use image::RgbaImage;
use std::{
    ffi::OsStr,
    fs::{self},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::channel,
    },
};
use walkdir::WalkDir;

use anyhow::{Result, anyhow};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

use crate::exporter::Exporter;

pub fn clear_dir(dir: &PathBuf) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    Ok(())
}

pub fn find_dlls(temp_path: &Path) -> anyhow::Result<Vec<String>> {
    let mut dlls = Vec::new();

    for entry in WalkDir::new(temp_path)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            if entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("dll"))
                .unwrap_or(false)
            {
                dlls.push(entry.path().to_string_lossy().to_string());
            }
        }
    }

    if dlls.is_empty() {
        anyhow::bail!("temp klasörü boş veya dll yok");
    }

    dlls.sort_by_key(|path| {
        let stem = Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let last_digit = stem
            .chars()
            .last()
            .and_then(|c| c.to_digit(10))
            .unwrap_or(999);
        (last_digit, stem)
    });

    Ok(dlls)
}
pub fn watch_and_copy(
    path: &PathBuf,
    out: &PathBuf,
    extension: &str,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    if path.is_file() {
        anyhow::bail!("watch path must be a DIRECTORY, but file given: {:?}", path);
    }

    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }

    println!("Watching: {:?}", path.display());

    let (tx, rx) = channel();

    let mut watcher: RecommendedWatcher = Watcher::new(tx, Config::default())?;

    watcher.watch(&path.canonicalize()?, RecursiveMode::Recursive)?;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv() {
            Ok(event) => {
                let event = event?;
                if let notify::EventKind::Modify(_) = event.kind {
                    let path = event
                        .paths
                        .first()
                        .ok_or(anyhow::anyhow!("No path found"))?;
                    if path.extension().and_then(OsStr::to_str) == Some(extension) {
                        let out_path = out.join(
                            path.file_name()
                                .ok_or_else(|| anyhow::anyhow!("Failed to get file name"))?,
                        );
                        println!("copy: {:?}", path);
                        if let Err(e) = std::fs::copy(path.canonicalize()?, &out_path) {
                            println!("Failed to copy {:?} -> {:?}: {:?}", path, out_path, e);
                        }
                    }
                }
            }
            Err(e) => println!("watch error: {:?}", e),
        }
    }
    Ok(())
}

pub fn take_screenshot(exporter: &Exporter, swf: &mut Vec<u8>) -> Result<Vec<RgbaImage>> {
    let movie_export = exporter.start_exporting_movie(swf)?;

    let mut result = Vec::new();
    let totalframes = movie_export.total_frames();

    for i in 0..4 {
        movie_export.run_frame();

        match movie_export.capture_frame() {
            Ok(image) => {
                println!("Capturing frame: {}", i);
                result.push(image)
            }
            Err(e) => {
                return Err(anyhow!("Unable to capture frame {} of: {:?}", i, e));
            }
        }
    }
    Ok(result)
}
