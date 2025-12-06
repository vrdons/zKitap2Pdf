use crate::paths;
use std::fs;
use std::io;
use std::path::Path;
use serde::Serialize;

pub fn setup() {
    let _ = fs::create_dir_all(paths::INPUT_PATH);
    let _ = fs::create_dir_all(paths::OUTPUT_PATH);
    let _ = fs::create_dir_all(paths::WINE_PATH);
    let _ = clear_temp();
}

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

fn clear_temp() -> anyhow::Result<()> {
    let tmp_dir = Path::new(paths::TEMP_PATH);

    if tmp_dir.exists() {
        fs::remove_dir_all(tmp_dir)?;
    }

    fs::create_dir_all(tmp_dir)?;

    Ok(())
}