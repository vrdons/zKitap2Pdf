use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    cli::Args,
    executable::{execute_exe, get_roaming_path, setup_environment},
    utils::{clear_dir, find_dlls, take_screenshot},
};

use anyhow::Result;
use clap::Parser;

pub mod cli;
pub mod executable;
pub mod exporter;
pub mod paths;
pub mod utils;

fn main() -> Result<()> {
    let arg = Args::parse();
    let (input, output, scale) = arg.validate()?;
    let temp_dir = Path::new(paths::TEMP_DIR);

    //Environment setup
    clear_dir(&temp_dir.to_path_buf())?;
    setup_environment()?;
    let exporter = exporter::Exporter::new(&exporter::Opt {
        graphics: arg.graphics,
        size: exporter::SizeOpt {
            width: 1132,
            height: 1614,
            scale,
        },
    })?;

    let stop_watch = Arc::new(AtomicBool::new(false));
    let roaming = get_roaming_path()?;

    let rc = roaming.clone();
    let tmp = temp_dir.to_path_buf().clone();
    let stp = stop_watch.clone();
    let _watcher = std::thread::spawn(move || {
        utils::watch_and_copy(&rc, &tmp, "dll", stp).unwrap_or_else(|e| println!("watch: {}", e))
    });
    execute_exe(&input)?.wait()?;

    //Sleeping for 5 seconds to allow the watcher to copy the files
    std::thread::sleep(Duration::from_millis(5000));
    stop_watch.store(true, Ordering::Relaxed);

    let dlls = find_dlls(&temp_dir)?;
    for dll in dlls {
        let mut read: Vec<u8> = fs::read(dll)?;
        let output = temp_dir;
        let frames = take_screenshot(&exporter, &mut read)?;
        let digits = frames.len().to_string().len();
        for (frame, image) in frames.iter().enumerate() {
            let mut path: PathBuf = (&output).into();
            path.push(format!("{frame:0digits$}.png"));
            image.save(&path)?;
        }
    }
    Ok(())
}
