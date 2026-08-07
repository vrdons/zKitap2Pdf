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
    /// When omitted it is resolved automatically: `--target-dpi` if given,
    /// otherwise the default 1.8.
    #[arg(short = 's', long)]
    pub scale: Option<f64>,

    /// Target print resolution (DPI) for rendered pages, e.g. 150 or 200.
    /// Converts to a scale factor as `dpi / 72` (Flash's design resolution).
    #[arg(long)]
    pub target_dpi: Option<u32>,

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
    /// Final resolved scale (from `--scale`, `--target-dpi`, or the default).
    pub scale: f64,
    /// Raw `--target-dpi` value, kept for diagnostics. `None` = not given
    /// (or overridden by `--scale`).
    pub target_dpi: Option<u32>,
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

        // Resolve the render scale: an explicit `--scale` wins and disables
        // `--target-dpi`; otherwise the default is 1.8. In `--target-dpi`
        // mode the scale is **not** fixed here — it is resolved per page at
        // render time (small pages are enlarged to the target, big pages are
        // left untouched), so `scale` is set to 1.0 as a neutral placeholder.
        let scale = match self.scale {
            Some(s) => {
                if self.target_dpi.is_some() {
                    eprintln!("warning: --scale given, ignoring --target-dpi");
                }
                s
            }
            None => {
                if self.target_dpi.is_some() {
                    1.0 // no fixed scale; resolved per page at render time
                } else {
                    1.8
                }
            }
        };
        // Carry the raw `--target-dpi` only when it is actually in effect
        // (i.e. no explicit `--scale`), so downstream stages never see a
        // disabled DPI value.
        let target_dpi = if self.scale.is_some() {
            None
        } else {
            self.target_dpi
        };
        if !(1.0..=3.0).contains(&scale) {
            bail!("scale must be between 1.0 and 3.0, got {scale}");
        }
        if let Some(dpi) = target_dpi
            && !(72..=300).contains(&dpi)
        {
            bail!("--target-dpi must be between 72 and 216, you dont wanna make 4k pdf image. got {dpi}");
        }

        let files = if self.input.is_dir() {
            self.collect_dir()?
        } else {
            vec![self.collect_single()?]
        };

        Ok(ValidatedArgs {
            files,
            scale,
            target_dpi,
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
