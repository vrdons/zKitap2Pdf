//! zKitap2Pdf entry point.
//!
//! Parses the CLI, initialises tracing, sets up the Wine environment, and
//! dispatches each input EXE to the conversion pipeline. Per-file errors are
//! collected (rather than aborting) so a batch run reports every failure.

mod cli;
mod config;
pub mod error;
mod fernus;
mod image_proc;
mod pdf;
mod pipeline;
mod ruffle;
mod utils;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use crate::cli::Args;
use crate::ruffle::exporter::{Exporter, ExporterOpt};
use crate::utils::logging;
use crate::utils::process::setup_environment;

fn main() -> Result<()> {
    let raw_args = Args::parse();
    logging::init(raw_args.debug);
    let args = raw_args.validate()?;

    // Configure the global rayon pool: `--cores 0` (default) uses all
    // available cores; an explicit value caps parallel page processing and
    // batch parallelism.
    let core_count = if args.cores == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        args.cores
    };
    if core_count > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(core_count)
            .build_global()
            .context("building rayon thread pool")?;
    }
    tracing::info!(cores = core_count, "rayon pool configured");

    let exporter = Exporter::new(&ExporterOpt {
        graphics: args.graphics,
        scale: args.scale,
        max_mem: args.max_mem,
    })?;

    setup_environment()?;

    let upscale = crate::image_proc::UpscaleOpts::new(args.scale);
    let mut errors = Vec::new();

    // v3 files (Enigma + Flutter) are fully CPU-bound and independent: they
    // parallelise trivially. v1/v2 files launch a Wine projector that drops
    // payloads into a *shared* %TEMP% watcher — running those concurrently
    // would mix up payloads, so they stay serial.
    let (v3_files, legacy_files): (Vec<_>, Vec<_>) =
        args.files.iter().partition(|f| crate::utils::has_enigma(&f.input));

    let v3_errors: Vec<_> = v3_files
        .par_iter()
        .filter_map(|file| {
            tracing::info!(input = %file.input.display(), "processing (v3)");
            if let Err(e) = pipeline::handle_exe(&exporter, file, &upscale, args.cores, args.max_mem)
            {
                tracing::error!(error = %e, input = %file.input.display(), "conversion failed");
                Some((file.input.clone(), e))
            } else {
                None
            }
        })
        .collect();
    errors.extend(v3_errors);

    for file in &legacy_files {
        tracing::info!(input = %file.input.display(), "processing (v1/v2)");
        if let Err(e) = pipeline::handle_exe(&exporter, file, &upscale, args.cores, args.max_mem) {
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
