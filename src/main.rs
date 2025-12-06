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
        let TempData = Path::new(paths::WINE_PATH)
            .join("drive_c")
            .join("users")
            .join(username)
            .join("AppData")
            .join("Local")
            .join("Temp");

        #[cfg(target_os = "linux")]
        let child = &mut utils::wine::run_file(&item.path).unwrap();
        //TODO: exec for windows

        let _ = child.wait();

        break;
        //panic!("{:?}", output);
    }
}
