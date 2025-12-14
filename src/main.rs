use std::time::Duration;

use crate::{cli::Args, executable::setup_environment, export::HandleArgs};

use clap::Parser;

pub mod cli;
pub mod executable;
pub mod export;
pub mod exporter;
pub mod utils;
pub mod crypto;

/// Parse command-line arguments, initialize the exporter and environment, process each input file, and report any per-file failures.

///

/// Processes each file from the parsed CLI arguments by invoking the export handler. Per-file errors are collected and printed after processing all files; the program still returns success unless an initialization step fails (argument validation, exporter construction, or environment setup).

///

/// # Examples

///

/// ```no_run

/// # use anyhow::Result;

/// # fn try_main() -> Result<()> { crate::main() }

/// # let _ = try_main();

/// ```
fn main() -> anyhow::Result<()> {
    let args = Args::parse().validate()?;
    let exporter = exporter::Exporter::new(&exporter::ExporterOpt {
        graphics: args.graphics,
        scale: args.scale,
    })?;
    setup_environment()?;
    let mut errors = Vec::new();

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