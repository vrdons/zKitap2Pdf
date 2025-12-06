use std::env;
#[cfg(target_os = "linux")]
use std::path::Path;

pub mod paths;
pub mod utils;

#[tokio::main]
async fn main() {
    #[cfg(target_os = "linux")]
    utils::wine::setup_wine().unwrap();

    utils::filesystem::setup();
    let files = utils::filesystem::check_input().unwrap_or_else(|e| {
        panic!("check_input hata verdi: {}", e);
    });
    if files.is_empty() {
        panic!("check_input: hiçbir dosya bulunamadı!");
    }
    for item in files {
        let username = env::var("USERNAME")
            .or_else(|_| env::var("USER"))
            .unwrap_or_else(|_| "unknown".to_string());
        #[cfg(target_os = "linux")]
        let TempData = utils::wine::get_temp_path().unwrap();

        let TempClone = TempData.clone();
        let handle = std::thread::spawn(move || utils::filesystem::watch_folder(&TempClone));

        #[cfg(target_os = "linux")]
        let child = &mut utils::wine::run_file(&item.path).unwrap();
        //TODO: exec for windows
        match handle.join() {
            Ok(result) => match result {
                Ok(zip_path) => {
                    println!("Thread’den gelen zip dosyası: {:?}", zip_path);
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
