use crate::paths;
use std::fs;
use std::io;
use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct FsEntry {
    pub path: String,
}

pub fn check_input() -> io::Result<Vec<FsEntry>> {
    let mut items = Vec::new();
    scan_dir(paths::INPUT_PATH, &mut items)?;
    Ok(items)
}

fn scan_dir<P: AsRef<std::path::Path>>(path: P, out: &mut Vec<FsEntry>) -> io::Result<()> {
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        let path_buf = entry.path();
        let path_str = path_buf.to_string_lossy().to_string();

        out.push(FsEntry { path: path_str.clone() });

        if entry.metadata()?.is_dir() {
            scan_dir(path_buf, out)?;
        }
    }
    Ok(())
}
