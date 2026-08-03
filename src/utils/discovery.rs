//! File discovery helpers (was `find_files` in `utils/mod.rs`).

use std::path::Path;

use anyhow::{Result, bail};
use walkdir::WalkDir;

/// Recursively collect every file under `path` whose extension matches
/// `extension` (case-insensitive, without the leading dot).
///
/// Errors reference the *actual* extension and path being searched — the old
/// code hardcoded a `"dll"` message even when scanning for `.exe` inputs.
pub fn find_files(path: &Path, extension: &str) -> Result<Vec<String>> {
    let mut file_paths = Vec::new();

    for entry in WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let is_match = entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension));
        if is_match {
            file_paths.push(entry.path().to_string_lossy().into_owned());
        }
    }

    if file_paths.is_empty() {
        bail!("no .{extension} files found in {}", path.display());
    }

    Ok(file_paths)
}
