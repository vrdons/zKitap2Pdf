use std::{ffi::OsStr, path::PathBuf};

use clap::Parser;
use ruffle_render_wgpu::clap::GraphicsBackend;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input file
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output file
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Scale factor for the image (bigger = better quality)
    #[clap(short = 's', long, default_value_t = 20 ,value_parser = clap::value_parser!(u64).range(10..=30))]
    pub scale: u64,

    #[clap(long, short, default_value = "default")]
    pub graphics: GraphicsBackend,
}

pub struct Files {
    pub input: PathBuf,
    pub output: PathBuf,
    pub filename: String,
}

impl Args {
    pub fn validate(&self) -> anyhow::Result<(Vec<Files>, f64)> {
        let mut list = Vec::new();

        if !self.input.exists() {
            anyhow::bail!("Input does not exist: {:?}", self.input);
        }

        if self.input.is_dir() {
            let files = crate::utils::find_files(self.input.clone().as_path(), "exe")?;
            let output = self.output.clone().unwrap_or_else(|| PathBuf::from("out"));

            if output.is_file() {
                anyhow::bail!("Output path must be a directory, not a file: {:?}", output);
            }

            if !output.exists() {
                std::fs::create_dir_all(&output)?;
            }

            if files.is_empty() {
                anyhow::bail!("No .exe files found in input directory");
            }

            for file in files {
                let input = std::fs::canonicalize(&file)?;
                let filename = input
                    .file_stem()
                    .ok_or_else(|| anyhow::anyhow!("Input file has no valid name: {:?}", input))?
                    .to_string_lossy()
                    .to_string();

                let output = output.join(format!("{}.pdf", filename));

                list.push(Files {
                    input,
                    output,
                    filename,
                });
            }
        } else {
            if self.input.extension().and_then(|e| e.to_str()) != Some("exe") {
                anyhow::bail!(
                    "Input must be a directory or an .exe file: {:?}",
                    self.input
                );
            }

            let input = std::fs::canonicalize(&self.input)?;
            let filename = input.file_stem().unwrap().to_string_lossy().to_string();
            let output = self
                .output
                .clone()
                .unwrap_or_else(|| PathBuf::from(format!("{}.pdf", filename)));

            if output.extension().and_then(OsStr::to_str) != Some("pdf") {
                anyhow::bail!("Output file must be a PDF: {:?}", output);
            }

            list.push(Files {
                input,
                output,
                filename,
            });
        }

        let scale = self.scale as f64 / 10.0;

        Ok((list, scale))
    }
}
