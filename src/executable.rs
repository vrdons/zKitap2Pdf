use anyhow::{Result, anyhow};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "linux")]
use std::fs;

#[cfg(target_os = "linux")]
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
        let status = Command::new("wine")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| anyhow!("Wine not installed or not found in PATH"))?;

        if !status.success() {
            return Err(anyhow!("Wine not installed or not found in PATH"));
        }

        let prefix = get_wineprefix();
        fs::create_dir_all(&prefix)?;

        let _ = Command::new("wineboot")
            .arg("--init")
            .env("WINEPREFIX", &prefix)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .status();
    }

    Ok(())
}

pub fn get_temp_path() -> Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    #[cfg(target_os = "linux")]
    {
        let wineprefix = get_wineprefix();
        Ok(wineprefix
            .join("drive_c")
            .join("users")
            .join(username)
            .join("AppData")
            .join("Local")
            .join("Temp"))
    }

    #[cfg(target_os = "windows")]
    {
        Ok(std::env::temp_dir())
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
