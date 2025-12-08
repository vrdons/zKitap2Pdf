use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    cli::Args,
    executable::{execute_exe, get_roaming_path, setup_environment},
    utils::clear_dir,
};

use anyhow::Result;
use clap::Parser;

pub mod cli;
pub mod executable;
pub mod paths;
pub mod utils;

fn main() -> Result<()> {
    let arg = Args::parse();
    let (input, output, scale) = arg.validate()?;
    let temp_dir = Path::new(paths::TEMP_DIR);

    //Environment setup
    clear_dir(temp_dir)?;
    setup_environment()?;

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

    Ok(())
}
