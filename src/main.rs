use core::panic;
use std::{fs, path::Path};

pub mod paths;
pub mod utils;

#[tokio::main]
async fn main() {
    utils::filesystem::setup();
    let files = utils::filesystem::check_input().unwrap_or_else(|e| {
        panic!("check_input hata verdi: {}", e);
    });
    if files.is_empty() {
        panic!("check_input: hiçbir dosya bulunamadı!");
    }
    for item in files {
        #[cfg(target_os = "linux")]
        utils::wine::setup_wine().unwrap();

        utils::filesystem::setup();
        #[cfg(target_os = "linux")]
        let temp_data = utils::wine::get_temp_path().unwrap();

        let temp_clone = temp_data.clone();
        let handle = std::thread::spawn(move || utils::filesystem::watch_folder(&temp_clone));

        #[cfg(target_os = "linux")]
        let child = &mut utils::wine::run_file(&item).unwrap();
        //TODO: exec for windows

        match handle.join() {
            Ok(result) => match result {
                Ok(zip_path) => {
                    println!("Thread’den gelen zip dosyası: {:?}", zip_path);
                    fs::copy(zip_path, Path::new(paths::TEMP_PATH).join("temp.zip"))
                        .unwrap_or_else(|e| {
                            panic!("Thread'den gelen zip dosyasını kopyalayamadım: {}", e)
                        });
                    child
                        .kill()
                        .unwrap_or_else(|e| println!("zKitap kapatılamadı: {}", e));
                }
                Err(e) => eprintln!("watch_folder hatası: {:?}", e),
            },
            Err(e) => eprintln!("Thread panic oldu: {:?}", e),
        }

        break;
        //panic!("{:?}", output);
    }
}
