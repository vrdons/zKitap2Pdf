use crate::paths;
use anyhow::{Context, Ok};
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc::channel;

pub fn setup() {
    let _ = fs::create_dir_all(paths::INPUT_PATH);
    let _ = fs::create_dir_all(paths::OUTPUT_PATH);
    let _ = clear_temp();
}

pub fn check_input() -> anyhow::Result<Vec<String>> {
    let mut items = Vec::new();
    scan_dir(paths::INPUT_PATH, &mut items)?;
    Ok(items)
}

pub fn watch_folder(path: &PathBuf) -> anyhow::Result<PathBuf> {
    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, notify::Config::default())?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;

    println!("Klasör izleniyor: {:?}", path);

    loop {
        let event = rx.recv()?.context("Event alınamadı")?;
        for path in event.paths {
            let ext = path.extension().unwrap_or(std::ffi::OsStr::new("unknown"));
            if ext == std::ffi::OsStr::new("zip") {
                return Ok(path);
            }
        }
    }
}

fn scan_dir<P: AsRef<std::path::Path>>(path: P, out: &mut Vec<String>) -> anyhow::Result<()> {
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        let path_buf = entry.path();
        let path_str = path_buf.to_string_lossy().to_string();

        out.push(path_str.clone());

        if entry.metadata()?.is_dir() {
            scan_dir(path_buf, out)?;
        }
    }
    Ok(())
}

fn clear_temp() -> anyhow::Result<()> {
    let tmp_dir = Path::new(paths::TEMP_PATH);

    if tmp_dir.exists() {
        fs::remove_dir_all(tmp_dir)?;
    }

    fs::create_dir_all(tmp_dir)?;

    Ok(())
}
