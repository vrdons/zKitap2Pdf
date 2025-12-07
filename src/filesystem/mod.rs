use std::{fs, path::Path};

use crate::paths;

pub mod exec;
pub mod notify;
pub mod utils;

pub fn setup_files() -> anyhow::Result<()> {
    let temp_path = Path::new(&paths::TEMP_PATH);
    utils::clear_folder(temp_path)?;
    fs::create_dir_all(paths::INPUT_PATH)?;
    fs::create_dir_all(paths::OUTPUT_PATH)?;
    utils::clear_folder(&exec::get_temp_path()?)?;
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        fs::create_dir_all(paths::WINE_PATH)?;
        let wp = Path::new(paths::WINE_PATH)
            .canonicalize()?
            .to_string_lossy()
            .to_string();
        Command::new("wine").arg("--version").spawn()?;
        let mut child = Command::new("wineboot").env("WINEPREFIX", wp).spawn()?;

        child.wait()?;
    }
    Ok(())
}
