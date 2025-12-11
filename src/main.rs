use std::time::Duration;

use crate::{cli::Args, executable::setup_environment, export::HandleArgs};

use clap::Parser;

pub mod cli;
pub mod executable;
pub mod export;
pub mod exporter;
pub mod utils;

fn main() -> anyhow::Result<()> {
    let args = Args::parse().validate()?;
    let exporter = exporter::Exporter::new(&exporter::ExporterOpt {
        graphics: args.graphics,
        scale: args.scale,
    })?;
    setup_environment()?;
    for file in &args.files {
        println!("Processing : {:?}", file.input);
        if let Err(e) = export::handle_exe(
            &exporter,
            HandleArgs {
                file: file.clone(),
                scale: args.scale,
            },
        ) {
            println!("An error occurred: {:?}", e);
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    Ok(())
}
