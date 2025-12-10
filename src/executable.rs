use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::{env, fs};

fn get_wineprefix() -> PathBuf {
    if let Ok(env_prefix) = env::var("WINEPREFIX") {
        return PathBuf::from(env_prefix);
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".wine")
}

pub fn setup_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        Command::new("wine")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| anyhow!("Wine not installed or not found in PATH"))?;

        let prefix = get_wineprefix();
        fs::create_dir_all(&prefix)?;

        Command::new("wineboot")
            .env("WINEPREFIX", &prefix)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()?
            .wait()?;
    }

    Ok(())
}

pub fn get_roaming_path() -> Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    #[cfg(target_os = "linux")]
    {
        let wineprefix = get_wineprefix();
        Ok(wineprefix
            .join("drive_c")
            .join("users")
            .join(username)
            .join("AppData")
            .join("Roaming"))
    }

    #[cfg(target_os = "windows")]
    {
        Ok(PathBuf::from("C:/")
            .join("Users")
            .join(username)
            .join("AppData")
            .join("Roaming"))
    }
}

pub fn execute_exe(path: &Path) -> Result<Child> {
    #[cfg(target_os = "linux")]
    {
        let prefix = get_wineprefix();
        Command::new("wine")
            .arg(path)
            .env("WINEPREFIX", &prefix)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("Failed to execute EXE via wine: {}", e))
    }

    #[cfg(target_os = "windows")]
    {
        Command::new(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("Failed to execute EXE: {}", e))
    }
}
