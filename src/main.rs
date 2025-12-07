use std::{
    fs::File,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub mod exporter;
pub mod filesystem;
pub mod paths;
pub mod utils;

fn main() -> anyhow::Result<()> {
    // filesystem::setup_files()?;
    let input_path = Path::new(paths::INPUT_PATH);
    let temp_path = Path::new(paths::TEMP_PATH);
    /*
        let stop_watch = Arc::new(AtomicBool::new(false));
        let files = filesystem::utils::scan_folder(input_path, true)?;

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
    */
    let dlls: Vec<String> = filesystem::utils::scan_folder(&temp_path, false)?
        .into_iter()
        .filter(|f| {
            Path::new(f)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext_str| ext_str.eq_ignore_ascii_case("dll"))
                .unwrap_or(false)
        })
        .collect();
    if dlls.is_empty() {
        return Err(anyhow::anyhow!("temp klasörü boş"));
    }
    for dll in dlls {
        let mut read = File::open(dll)?;
        //   swf::handle_swf(&mut read)?;
    }
    /*
    break;
    //panic!("{:?}", output);
    }
     */
    Ok(())
}
