//! Project-wide constants and tuning knobs.

use std::time::Duration;

#[allow(dead_code)]
pub const WINE_MISSING: &str = "Wine not installed or not found in PATH";

/// DLL watcher (notify-based) timing knobs.
///
/// `IDLE_TIMEOUT` — once the first DLL is seen, stop when no new activity for
/// this long (projector has finished dropping files).
/// `MAX_TIMEOUT`   — hard ceiling on total wait regardless of activity (safety net).
/// `POLL_INTERVAL` — how often the watcher channel is drained.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(120);
pub const POLL_INTERVAL: Duration = Duration::from_millis(900);

/// Author string embedded in every generated PDF.
pub const PDF_AUTHOR: &str = "Vrdons <vrdons@proton.me>";
