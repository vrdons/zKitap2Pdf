use std::{io::Cursor, path::Path};
use swf::{Header, Rectangle, SwfBuf, Twips, write::write_swf_raw_tags};

use anyhow::Result;
use walkdir::WalkDir;

pub fn find_files(path: &Path, extension: &str) -> anyhow::Result<Vec<String>> {
    let mut file_paths = Vec::new();

    for entry in WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        {
            file_paths.push(entry.path().to_string_lossy().to_string());
        }
    }

    if file_paths.is_empty() {
        anyhow::bail!("temp folder is empty or dll not found");
    }

    Ok(file_paths)
}

pub fn patch_swf(file: SwfBuf) -> Result<Vec<u8>> {
    let header = Header {
        version: file.header.version(),
        compression: file.header.compression(),
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(566.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(807.0),
        },
        frame_rate: file.header.frame_rate(),
        num_frames: file.header.num_frames(),
    };
    let mut out = Cursor::new(Vec::<u8>::new());
    write_swf_raw_tags(&header, &file.data, &mut out)?;
    Ok(out.into_inner())
}
