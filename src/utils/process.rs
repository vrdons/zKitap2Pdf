//! Process execution + environment setup (was `utils/exec.rs`).
//!
//! Linux path: launch the Fernus projector under Wine, collect the runtime
//! DLLs dropped into the Wine prefix's `%TEMP%`.
//! Windows path: launch the projector directly, collect from `%TEMP%`.


use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use std::fs;
use std::env;
use crate::config::WINE_MISSING;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!("zKitap2Pdf only supports Linux (via Wine) and Windows; see issue #7");


/// Resolve the Wine prefix used by the current environment.
#[cfg(target_os = "linux")]
fn wineprefix() -> PathBuf {
    if let Ok(env_prefix) = env::var("WINEPREFIX") {
        return PathBuf::from(env_prefix);
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".wine")
}

pub fn setup_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let version_output = Command::new("wine")
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| anyhow::anyhow!(WINE_MISSING))?;

        if !version_output.status.success() {
            return Err(anyhow::anyhow!(WINE_MISSING));
        }

        let prefix = wineprefix();
        tracing::debug!(prefix = %prefix.display(), "resolved wine prefix");

        fs::create_dir_all(&prefix)
            .with_context(|| format!("creating wine prefix {}", prefix.display()))?;

        tracing::info!("initialising wine prefix (this may take a while)");
        let init_status = Command::new("wineboot")
            .arg("--init")
            .env("WINEPREFIX", &prefix)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .status();
        match init_status {
            Ok(status) => tracing::debug!(code = status.code(), "wineboot --init finished"),
            Err(e) => {
                tracing::warn!(error = %e, "wineboot --init failed");
            }
        }
    }

    Ok(())
}

/// Return the temporary directory that contains the application assets.
pub fn temp_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;
        Ok(wineprefix()
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
        let prefix = wineprefix();
        tracing::debug!(
            input = %path.display(),
            prefix = %prefix.display(),
            "launching projector via wine"
        );
        Command::new("wine")
            .arg(path)
            .env("WINEPREFIX", &prefix)
            //.stdout(Stdio::null())
            //.stderr(Stdio::null())
            //.stdin(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to execute EXE via wine: {}", path.display()))
    }

    #[cfg(target_os = "windows")]
    {
        tracing::debug!(input = %path.display(), "launching projector directly");
        Command::new(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to execute EXE: {}", path.display()))
    }
}
