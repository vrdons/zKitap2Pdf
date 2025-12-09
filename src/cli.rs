use std::path::PathBuf;

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

impl Args {
    pub fn validate(&self) -> anyhow::Result<(PathBuf, PathBuf, f64)> {
        if self.input.extension().and_then(|e| e.to_str()) != Some("exe") {
            anyhow::bail!("Input file is not an executable: {:?}", self.input);
        }

        if !self.input.exists() {
            anyhow::bail!("Input file does not exist: {:?}", self.input);
        }
        let output = self
            .output
            .clone()
            .unwrap_or_else(|| PathBuf::from("output.pdf"));

        if output.exists() {
            anyhow::bail!("Output file already exists: {:?}", output);
        }

        let input = std::fs::canonicalize(&self.input)?;

        let scale = self.scale as f64 / 10.0;

        Ok((input, output, scale))
    }
}
