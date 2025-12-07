use std::fs::File;

pub mod utils;

pub fn handle_swf(file: &mut File) -> anyhow::Result<()> {
    let swf = utils::decrypt_cws(file)?;
    Ok(())
}
