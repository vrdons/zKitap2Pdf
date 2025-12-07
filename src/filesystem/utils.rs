use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::bail;

pub fn scan_dir(dir: &Path, recursive: bool) -> anyhow::Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path_buf = entry.path();
        let path_str = path_buf.to_string_lossy().to_string();

        files.push(path_str);

        if entry.metadata()?.is_dir() && recursive {
            let recursive_scan = scan_dir(&path_buf, recursive)?;
            files.extend(recursive_scan);
        }
    }
    if files.is_empty() {
        return Err(anyhow::anyhow!("{} klasörü boş", dir.display()));
    }
    Ok(files)
}

pub fn clear_folder(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }

    fs::create_dir_all(path)?;
    Ok(())
}

pub fn find_closest_folder(base: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let mut best: Option<(PathBuf, f32)> = None;

    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        let sim = crate::utils::simple_similarity(&name, target);

        match &best {
            None => best = Some((entry.path(), sim)),
            Some((_, best_sim)) => {
                if sim > *best_sim {
                    best = Some((entry.path(), sim));
                }
            }
        }
    }

    match best {
        Some((path, _)) => Ok(path),
        None => bail!("cannot find closest folder for {}", base.display()),
    }
}
