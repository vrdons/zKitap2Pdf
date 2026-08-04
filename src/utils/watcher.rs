use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use notify::event::{AccessKind, AccessMode, EventKind};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{IDLE_TIMEOUT, MAX_TIMEOUT, POLL_INTERVAL};

/// Watch `temp_path` until DLLs stop appearing, then return them keyed by name.
///
/// Stops on the first of: idle (no new files for [`IDLE_TIMEOUT`]),
/// [`MAX_TIMEOUT`] hard ceiling, or watcher channel disconnect.
pub fn watch_and_collect(temp_path: &Path) -> Result<HashMap<String, Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, Config::default()).map_err(|e| anyhow!("notify init: {e}"))?;
    watcher
        .watch(temp_path, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", temp_path.display()))?;

    let mut dlls: HashMap<String, Vec<u8>> = HashMap::new();
    let mut last_activity = Instant::now();
    let start = Instant::now();

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(event)) => handle_event(&event, &mut dlls, &mut last_activity),
            Ok(Err(e)) => tracing::debug!(error = %e, "notify event error"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if should_stop(&dlls, &last_activity, start) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(dlls)
}

fn handle_event(
    event: &notify::Event,
    dlls: &mut HashMap<String, Vec<u8>>,
    last_activity: &mut Instant,
) {
    for path in &event.paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut in_process = false;

        if (name.ends_with(".dll") || name.ends_with(".frns") || name.ends_with(".kxk"))
            && event.kind == EventKind::Access(AccessKind::Close(AccessMode::Write))
        {
            in_process = true;
        }

        let owned = name.to_string();

        //tracing::trace!(name = %owned, action = ?event.kind);

        if !in_process {
            continue;
        }

        match fs::read(path) {
            Ok(data) => {
                let kb = format!("{:.1}", data.len() as f64 / 1024.0);
                let prev = dlls.get(&owned).map(|v| v.len()).unwrap_or(0);
                if data.len() != prev {
                    tracing::debug!(name = %owned, kb = %kb, prev_kb = format!("{:.1}", prev as f64 / 1024.0), "dll read");
                }
                dlls.insert(owned, data);
                *last_activity = Instant::now();
            }
            Err(e) => {
                if !dlls.contains_key(&owned) {
                    tracing::debug!(error = %e, name = %owned, "temp read failed");
                }
            }
        }
    }
}

fn should_stop(dlls: &HashMap<String, Vec<u8>>, last_activity: &Instant, start: Instant) -> bool {
    if !dlls.is_empty() && last_activity.elapsed() >= IDLE_TIMEOUT {
        tracing::debug!(reason = "idle", "stopping watcher");
        return true;
    }
    if start.elapsed() >= MAX_TIMEOUT {
        tracing::debug!(reason = "max-timeout", "stopping watcher");
        return true;
    }
    false
}
