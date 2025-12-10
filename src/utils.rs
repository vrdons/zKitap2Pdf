use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::channel},
};
use swf::{Header, Rectangle, Twips, write::write_swf_raw_tags};

use anyhow::Result;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use walkdir::WalkDir;
pub type FileBuffer = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

pub fn find_files(path: &Path, extension: &str) -> anyhow::Result<Vec<String>> {
    let mut file_paths = Vec::new();

    for entry in WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        {
            file_paths.push(entry.path().to_string_lossy().to_string());
        }
    }

    if file_paths.is_empty() {
        anyhow::bail!("temp folder is empty or dll not found");
    }

    Ok(file_paths)
}
pub fn watch_file(path: &PathBuf, buffer: FileBuffer) -> Result<()> {
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
        match rx.recv() {
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

                if filename == "p" {
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

                println!("found file: {:?}", file_path);

                let mut lock = buffer.lock().unwrap();

                if let Some(entry) = lock.iter_mut().find(|(n, _)| *n == filename) {
                    entry.1 = bytes;
                } else {
                    lock.push((filename, bytes));
                }
            }

            Err(e) => {
                println!("watch error: {:?}", e);
            }
        }
    }
}

pub fn patch_swf(file: &[u8]) -> Result<Vec<u8>> {
    let mut data = Cursor::new(file);
    let dec = swf::decompress_swf(&mut data)?;
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
    Ok(out.into_inner())
}

pub fn sort_files(lock: &mut [(String, Vec<u8>)]) {
    lock.sort_by_key(|(name, _data)| {
        let stem = Path::new(name)
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
}
