use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command},
};

use anyhow::{Context, Ok};

use crate::paths;

pub fn setup_wine() -> anyhow::Result<()> {
    let _ = fs::create_dir_all(paths::WINE_PATH);
    let wp = Path::new(paths::WINE_PATH)
        .canonicalize()?
        .to_string_lossy()
        .to_string();
    Command::new("wine")
        .arg("--version")
        .spawn()
        .unwrap_or_else(|e| panic!("Wine bulunamadı: {}", e));
    clear_temp()?;
    let mut child = Command::new("wineboot").env("WINEPREFIX", wp).spawn()?;

    let status = child.wait()?;

    if status.success() {
        println!("Wineboot başarıyla çalıştı.");
    } else {
        anyhow::bail!("Wineboot çalıştırılamadı! Çıkış kodu: {:?}", status.code());
    }

    Ok(())
}
pub fn run_file(path: &str) -> anyhow::Result<Child> {
    let wp = Path::new(paths::WINE_PATH)
        .canonicalize()?
        .to_string_lossy()
        .to_string();
    println!("{}", wp);
    let child = Command::new("wine")
        .env("WINEPREFIX", wp)
        .arg(path)
        .spawn()?;
    Ok(child)
}

pub fn get_temp_path() -> anyhow::Result<PathBuf> {
    let username = env::var("USERNAME")
        .or_else(|_| env::var("USER"))
        .context("Kullanıcı adı alınamadı")?;

    let tmp_dir = Path::new(paths::WINE_PATH)
        .join("drive_c")
        .join("users")
        .join(&username)
        .join("AppData")
        .join("Local")
        .join("Temp");

    Ok(tmp_dir)
}

fn clear_temp() -> anyhow::Result<()> {
    let tmp_dir = get_temp_path()?;

    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir)?;
    }

    fs::create_dir_all(&tmp_dir)?;
    Ok(())
}
