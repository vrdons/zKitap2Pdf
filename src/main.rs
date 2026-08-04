//! zKitap2Pdf entry point.
//!
//! Parses the CLI, initialises tracing, sets up the Wine environment, and
//! dispatches each input EXE to the conversion pipeline. Per-file errors are
//! collected (rather than aborting) so a batch run reports every failure.

mod cli;
mod config;
mod enigma;
pub mod error;
mod fernus;
mod image_proc;
mod pdf;
mod pipeline;
mod ruffle;
mod utils;

use anyhow::Result;
use clap::Parser;

use crate::cli::Args;
use crate::ruffle::exporter::{Exporter, ExporterOpt};
use crate::utils::logging;
use crate::utils::process::setup_environment;

fn main() -> Result<()> {
    let raw_args = Args::parse();
    logging::init(raw_args.debug);
    let args = raw_args.validate()?;

    let exporter = Exporter::new(&ExporterOpt {
        graphics: args.graphics,
        scale: args.scale,
    })?;

    setup_environment()?;

    let upscale = crate::image_proc::UpscaleOpts::new(args.scale);
    let mut errors = Vec::new();
    for file in &args.files {
        tracing::info!(input = %file.input.display(), "processing");
        if let Err(e) = pipeline::handle_exe(&exporter, file, &upscale) {
            tracing::error!(error = %e, input = %file.input.display(), "conversion failed");
            errors.push((file.input.clone(), e));
        }
    }

    if !errors.is_empty() {
        tracing::error!(count = errors.len(), "failed to process file(s)");
        for (path, e) in &errors {
            tracing::error!(input = %path.display(), error = ?e);
        }
    }
    Ok(())
}
