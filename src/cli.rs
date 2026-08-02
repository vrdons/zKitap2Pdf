use std::{ffi::OsStr, path::PathBuf};

use clap::Parser;
use ruffle_render_wgpu::clap::GraphicsBackend;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input file
    #[arg(value_name = "INPUT")]
    pub input: PathBuf,

    /// Output file
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Scale factor for the image (bigger = better quality)
    #[clap(short = 's', long, default_value_t = 28, value_parser = clap::value_parser!(u64).range(10..=30))]
    pub scale: u64,

    #[clap(long, short, default_value = "default")]
    pub graphics: GraphicsBackend,

    /// Enable verbose Temp folder watching output
    #[clap(long)]
    pub debug: bool,
}

#[derive(Clone, Debug)]
pub struct Files {
    pub input: PathBuf,
    pub output: PathBuf,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct ValidatedArgs {
    pub files: Vec<Files>,
    pub scale: f64,
    pub graphics: GraphicsBackend,
    pub debug: bool,
}

impl Args {
    pub fn validate(&self) -> anyhow::Result<ValidatedArgs> {
        if !self.input.exists() {
            anyhow::bail!("Input does not exist: {:?}", self.input);
        }

        let files = if self.input.is_dir() {
            self.collect_dir()?
        } else {
            vec![self.collect_single()?]
        };

        Ok(ValidatedArgs {
            files,
            scale: self.scale as f64 / 10.0,
            graphics: self.graphics,
            debug: self.debug,
        })
    }

    fn collect_dir(&self) -> anyhow::Result<Vec<Files>> {
        let found = crate::utils::find_files(&self.input, "exe")?;
        let output = self.output.clone().unwrap_or_else(|| PathBuf::from("out"));
        if output.is_file() {
            anyhow::bail!("Output path must be a directory, not a file: {:?}", output);
        }
        if !output.exists() {
            std::fs::create_dir_all(&output)?;
        }
        if found.is_empty() {
            anyhow::bail!("No .exe files found in input directory");
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

    fn collect_single(&self) -> anyhow::Result<Files> {
        if self.input.extension().and_then(|e| e.to_str()) != Some("exe") {
            anyhow::bail!(
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
            anyhow::bail!("Output file must be a PDF: {:?}", output);
        }
        Ok(Files {
            input,
            output,
            filename,
        })
    }
}

fn file_stem_string(path: &PathBuf) -> anyhow::Result<String> {
    path.file_stem()
        .ok_or_else(|| anyhow::anyhow!("Input file has no valid name: {:?}", path))
        .map(|s| s.to_string_lossy().into_owned())
}
