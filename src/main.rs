use std::{env, fs, path::Path, thread, time::Duration};

pub mod paths;
pub mod utils;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    utils::filesystem::setup();
    let files = utils::filesystem::check_input()
        .map_err(|e| anyhow::anyhow!("check_input hat verdi: {}", e))?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("input klasörü boş"));
    }
    for item in files {
        #[cfg(target_os = "linux")]
        utils::wine::setup_wine()?;
        let username = env::var("USERNAME").or_else(|_| env::var("USER"))?;
        utils::filesystem::setup();
        #[cfg(target_os = "linux")]
        let temp_data = utils::wine::get_temp_path()?;

        let temp_clone = temp_data.clone();
        let temp_clone2 = Path::new(paths::WINE_PATH)
            .join("drive_c")
            .join("users")
            .join(&username)
            .join("AppData")
            .join("Roaming")
            .join("isler-vaf-2022-11-sinif-matematik-vaf-6")
            .join("Local Store")
            .join("#SharedObjects")
            .clone();
        let handle_zip =
            std::thread::spawn(move || utils::filesystem::watch_folder(&temp_clone, "zip"));
        let handle_kxk =
            std::thread::spawn(move || utils::filesystem::watch_folder(&temp_clone2, "dll"));

        #[cfg(target_os = "linux")]
        let child = &mut utils::wine::run_file(&item)?;
        //TODO: exec for windows
        let zip_path = handle_zip
            .join()
            .map_err(|e| anyhow::anyhow!("Thread panic oldu: {:?}", e))??;

        println!("Thread’den gelen zip dosyası: {:?}", zip_path);
        thread::sleep(Duration::from_millis(100));
        utils::zip::extract_zip(zip_path, Path::new(paths::TEMP_PATH).join("temp")).await?;

        let kxk_path = handle_kxk
            .join()
            .map_err(|e| anyhow::anyhow!("Thread panic oldu: {:?}", e))??;
        child
            .kill()
            .map_err(|e| anyhow::anyhow!("zKitap kapatılamadı: {}", e))?;
        let file = fs::read(kxk_path)?;
        let pass = "pub1isher1l0O";
        //    utils::crypto::decrypt_publisher(file, pass);
        break;
        //panic!("{:?}", output);
    }
    Ok(())
}
