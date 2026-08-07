//! CLI argument parsing and validation (was `utils/cli.rs`).
//!
//! `Args` is the clap-derived raw input. `ValidatedArgs` is the canonicalised
//! form consumed by the pipeline. We intentionally do **not** re-bundle into a
//! per-file `HandleArgs` anymore — the pipeline takes `(exporter, &Files,
//! scale)` directly, since `debug` becomes global once tracing is initialised.

use std::{ffi::OsStr, path::PathBuf};

use anyhow::{Result, anyhow, bail};
use clap::Parser;
use ruffle_render_wgpu::clap::GraphicsBackend;

/// CLI options accepted by the binary.
#[derive(Parser, Debug)]
#[command(name = "zKitap2Pdf", version, about)]
pub struct Args {
    /// Input file or directory.
    #[arg(value_name = "INPUT")]
    pub input: PathBuf,

    /// Output file (single EXE) or directory (batch).
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Scale factor applied to the rendered image (1.0–3.0, e.g. 2.8 = 280%).
    #[arg(short = 's', long, default_value_t = 1.8)]
    pub scale: f64,

    /// Graphics backend used by Ruffle.
    #[arg(long, short, default_value = "default")]
    pub graphics: GraphicsBackend,

    /// Number of CPU cores to use for parallel page processing.
    /// `0` (default) = auto-detect (all available cores).
    #[arg(long, default_value_t = 0)]
    pub cores: usize,

    /// Approximate memory budget (MiB) for in-flight pages.
    /// `0` (default) = unbounded (chunk size = core count, capped at 8).
    #[arg(long, default_value_t = 0)]
    pub max_mem: usize,

    /// Enable verbose output (debug-level logs).
    #[arg(long)]
    pub debug: bool,
}

/// Normalized input/output information for a single EXE.
#[derive(Clone, Debug)]
pub struct Files {
    pub input: PathBuf,
    pub output: PathBuf,
    pub filename: String,
}

/// Parsed and validated arguments.
#[derive(Debug, Clone)]
pub struct ValidatedArgs {
    pub files: Vec<Files>,
    pub scale: f64,
    pub graphics: GraphicsBackend,
    pub cores: usize,
    pub max_mem: usize,
}

impl Args {
    /// Canonicalise raw CLI input into a [`ValidatedArgs`].
    ///
    /// `debug` is intentionally **not** carried here — it is consumed once at
    /// startup to initialise tracing, and from then on log level is global.
    pub fn validate(&self) -> Result<ValidatedArgs> {
        if !self.input.exists() {
            bail!("Input does not exist: {:?}", self.input);
        }
        if !(1.0..=3.0).contains(&self.scale) {
            bail!("scale must be between 1.0 and 3.0, got {}", self.scale);
        }

        let files = if self.input.is_dir() {
            self.collect_dir()?
        } else {
            vec![self.collect_single()?]
        };

        Ok(ValidatedArgs {
            files,
            scale: self.scale,
            graphics: self.graphics,
            cores: self.cores,
            max_mem: self.max_mem,
        })
    }

    fn collect_dir(&self) -> Result<Vec<Files>> {
        let found = crate::utils::discovery::find_files(&self.input, "exe")?;
        let output = self.output.clone().unwrap_or_else(|| PathBuf::from("out"));
        if output.is_file() {
            bail!("Output path must be a directory, not a file: {:?}", output);
        }
        if !output.exists() {
            std::fs::create_dir_all(&output)
                .map_err(|e| anyhow!("creating output dir {}: {e}", output.display()))?;
        }
        if found.is_empty() {
            bail!("No .exe files found in input directory");
        }

        let mut list = Vec::with_capacity(found.len());
        for f in found {
            let input = std::fs::canonicalize(&f)?;
            let filename = file_stem_string(&input)?;
            let output = output.join(format!("{filename}.pdf"));
            list.push(Files {
                input,
                output,
                filename,
            });
        }
        Ok(list)
    }

    fn collect_single(&self) -> Result<Files> {
        if self.input.extension().and_then(|e| e.to_str()) != Some("exe") {
            bail!(
                "Input must be a directory or an .exe file: {:?}",
                self.input
            );
        }
        let input = std::fs::canonicalize(&self.input)?;
        let filename = file_stem_string(&input)?;
        let output = self
            .output
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{filename}.pdf")));
        if output.extension().and_then(OsStr::to_str) != Some("pdf") {
            bail!("Output file must be a PDF: {:?}", output);
        }
        Ok(Files {
            input,
            output,
            filename,
        })
    }
}

fn file_stem_string(path: &PathBuf) -> Result<String> {
    path.file_stem()
        .ok_or_else(|| anyhow!("Input file has no valid name: {:?}", path))
        .map(|s| s.to_string_lossy().into_owned())
}
