use std::{
    fs::File,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::filesystem::setup_files;

pub mod filesystem;
pub mod paths;
pub mod utils;

fn main() -> anyhow::Result<()> {
    setup_files()?;
    let input_path = Path::new(paths::INPUT_PATH);
    let temp_path = Path::new(paths::TEMP_PATH);

    let stop_watch = Arc::new(AtomicBool::new(false));
    let files = filesystem::utils::scan_dir(input_path, true)?;

    for item in files {
        println!("Executing app: {:#?}", item.clone());
        let roaming = filesystem::exec::get_roaming_path(item.clone())?;
        println!("App Roaming Path: {:#?}", &roaming);
        println!("Wrong path? try executing once");

        let rc = roaming.clone();
        let tmp = temp_path.to_path_buf().clone();
        let stp = stop_watch.clone();
        let _inotify = std::thread::spawn(move || {
            filesystem::notify::watch_and_copy(&rc, &tmp, "dll", stp)
                .unwrap_or_else(|e| println!("watch: {}", e))
        });
        let child = &mut filesystem::exec::execute_exe(&item)?;
        child.wait()?;
        println!("Child finished");
        std::thread::sleep(Duration::from_millis(5000));
        stop_watch.store(true, Ordering::Relaxed);
        break;
        //panic!("{:?}", output);
    }
    Ok(())
}
/*
let stop = Arc::new(AtomicBool::new(false));
let stop2 = stop.clone();
// &temp_clone, "dll", stop2
let _ = std::thread::spawn(move || {
    utils::inotify::watch_and_copy(&roaming.clone())
        .unwrap_or_else(|e| panic!("inotify: {}", e))
});
#[cfg(target_os = "linux")]
let child = &mut utils::wine::run_file(&item)?;
//TODO: exec for windows
child.wait()?;
println!("Child finished");
std::thread::sleep(Duration::from_millis(5000));
stop.store(true, Ordering::Relaxed);

let dlls = check_files(&temp.to_string_lossy().to_string())?;
if dlls.is_empty() {
    return Err(anyhow::anyhow!("temp klasörü boş"));
}
for dll in dlls {
    let mut read = File::open(dll)?;
    extract_cws(&mut read)?;
}
*/
