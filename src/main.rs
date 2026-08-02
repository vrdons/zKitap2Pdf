use std::time::Duration;

use crate::cli::Args;
use crate::executable::setup_environment;
use crate::export::HandleArgs;

use clap::Parser;

pub mod cli;
pub mod decrypt;
pub mod executable;
pub mod export;
pub mod exporter;
pub mod fernus_assets;
pub mod pe_scanner;
pub mod utils;

fn main() -> anyhow::Result<()> {
    let args = Args::parse().validate()?;
    let exporter = exporter::Exporter::new(&exporter::ExporterOpt {
        graphics: args.graphics,
        scale: args.scale,
    })?;
    println!("-- Setting up environment, this may take a while...");
    setup_environment()?;
    let mut errors = Vec::new();

    for file in &args.files {
        println!("Processing: {:?}", file.input);
        if let Err(e) = export::handle_exe(
            &exporter,
            &HandleArgs {
                file: file.clone(),
                scale: args.scale,
                debug: args.debug,
            },
        ) {
            println!("An error occurred: {:?}", e);
            errors.push((file.input.clone(), e));
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    if !errors.is_empty() {
        eprintln!("Failed to process {} file(s)", errors.len());
        eprintln!("{:?}", errors);
    }
    Ok(())
}
