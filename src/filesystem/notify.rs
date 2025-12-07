use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;

pub fn watch_and_copy(
    path: &PathBuf,
    out: &PathBuf,
    extension: &str,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    std::fs::create_dir_all(path)?;

    println!("Watching: {:?}", path.display());

    let (tx, rx) = channel();

    let mut watcher: RecommendedWatcher = Watcher::new(tx, notify::Config::default())?;

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
                        println!("copying: {:#?}", path.file_name());
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
