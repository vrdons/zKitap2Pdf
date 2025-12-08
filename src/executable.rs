use crate::paths;
use anyhow::Result;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::{
    env, fs,
    path::PathBuf,
    process::{Child, Command},
};

pub fn setup_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;

        fs::create_dir_all(paths::WINE_PATH)?;
        let wine_path = Path::new(paths::WINE_PATH)
            .canonicalize()?
            .to_string_lossy()
            .to_string();
        Command::new("wine").arg("--version").spawn()?;
        let mut child = Command::new("wineboot")
            .env("WINEPREFIX", wine_path)
            .spawn()?;

        child.wait()?;
    }
    Ok(())
}

pub fn get_temp_path() -> anyhow::Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    #[cfg(target_os = "linux")]
    let tmp = Path::new(crate::paths::WINE_PATH)
        .join("drive_c")
        .join("users")
        .join(&username)
        .join("AppData")
        .join("Local")
        .join("Temp");

    Ok(tmp)
}

pub fn get_roaming_path() -> anyhow::Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    #[cfg(target_os = "linux")]
    let roaming = Path::new(crate::paths::WINE_PATH)
        .join("drive_c")
        .join("users")
        .join(username)
        .join("AppData")
        .join("Roaming");

    Ok(roaming)
}

pub fn execute_exe(path: &PathBuf) -> anyhow::Result<Child> {
    #[cfg(target_os = "linux")]
    {
        use std::process::Stdio;

        let wp = Path::new(crate::paths::WINE_PATH)
            .canonicalize()?
            .to_string_lossy()
            .to_string();
        let child = Command::new("wine")
            .env("WINEPREFIX", wp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .arg(path)
            .spawn()?;
        Ok(child)
    }
}
