use std::{
    env,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub fn get_temp_path() -> anyhow::Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    #[cfg(target_os = "linux")]
    let tmp_dir = Path::new(crate::paths::WINE_PATH)
        .join("drive_c")
        .join("users")
        .join(&username)
        .join("AppData")
        .join("Local")
        .join("Temp");

    Ok(tmp_dir)
}

pub fn get_roaming_path(appname: String) -> anyhow::Result<PathBuf> {
    let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;

    let real_app_name = Path::new(&appname)
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("Geçersiz uygulama adı"))?
        .to_string_lossy()
        .into_owned();

    #[cfg(target_os = "linux")]
    let roaming_base = Path::new(crate::paths::WINE_PATH)
        .join("drive_c")
        .join("users")
        .join(username)
        .join("AppData")
        .join("Roaming");

    let closest = super::utils::find_closest_folder(&roaming_base, &real_app_name)?;

    Ok(closest)
}
pub fn execute_exe(path: &String) -> anyhow::Result<Child> {
    #[cfg(target_os = "linux")]
    {
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
