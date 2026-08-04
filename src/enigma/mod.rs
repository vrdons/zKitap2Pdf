//! Enigma Virtual Box PE section parser and VFS extractor.
mod pe;
mod records;
mod aplib;

use std::fs;

pub use pe::PeInfo;

/// Kind of a VFS entry.
#[derive(Debug, Clone, PartialEq)]
pub enum VfsEntryKind {
    File,
    Folder,
}

#[derive(Debug, Clone)]
pub struct VfsEntry {
    pub path: String,
    pub kind: VfsEntryKind,
    pub offset: u64,
    pub stored_size: u32,
    pub original_size: u32,
}

#[derive(Debug)]
pub struct ExtractedFile {
    pub path: String,
    pub data: Vec<u8>,
}

pub fn unpack(exe_path: &std::path::Path) -> Result<Vec<ExtractedFile>, Error> {
    let raw = fs::read(exe_path)?;
    let pe = PeInfo::parse(&raw)?;

    // Find .enigma1 section
    let enigma1_info = pe
        .sections
        .iter()
        .find(|s| s.name == ".enigma1")
        .ok_or(Error::NoEnigmaSection(".enigma1".into()))?;

    let enigma1_start = enigma1_info.raw_offset as usize;
    let enigma1_end = enigma1_start + enigma1_info.raw_size as usize;
    if enigma1_end > raw.len() {
        return Err(Error::UnexpectedEof);
    }
    let enigma1_data = &raw[enigma1_start..enigma1_end];

    let magic_offset = enigma1_data
        .windows(4)
        .position(|w| w == b"EVB\x00")
        .ok_or(Error::InvalidMagic)?;

    let vfs_data = &enigma1_data[magic_offset..];
    let entries = records::parse_vfs_tree(vfs_data)?;

    let base_offset = (enigma1_start + magic_offset) as u64;

    let mut results = Vec::new();
    for entry in &entries {
        if entry.kind != VfsEntryKind::File {
            continue;
        }

        let abs_offset = base_offset + entry.offset;
        let end_offset = abs_offset + entry.stored_size as u64;

        if end_offset > raw.len() as u64 {
            tracing::warn!(
                path = %entry.path,
                offset = abs_offset,
                size = entry.stored_size,
                "VFS file extends beyond EXE, skipping"
            );
            continue;
        }

        let data = if entry.stored_size != entry.original_size {
            decompress_chunks(&raw, abs_offset, entry.stored_size, entry.original_size)?
        } else {
            raw[abs_offset as usize..end_offset as usize].to_vec()
        };

        results.push(ExtractedFile {
            path: entry.path.clone(),
            data,
        });
    }

    Ok(results)
}

/// Decompress a file stored in EVB chunk format (aPLib).
///
/// Layout at `offset`:
/// ```text
/// EVB_CHUNK_BLOCK { size: u32, padding: u32 }    (8 bytes)
/// chunk size table  [u32; (size - 8) / 4]          (12 bytes per entry, every 3rd is chunk size)
/// compressed chunk 1
/// compressed chunk 2
/// ...
/// ```
fn decompress_chunks(
    raw: &[u8],
    offset: u64,
    stored_size: u32,
    original_size: u32,
) -> Result<Vec<u8>, Error> {
    let start = offset as usize;
    let block = &raw[start..start + stored_size as usize];

    if block.len() < 8 {
        return Err(Error::VfsParse("chunk block too small".into()));
    }

    // Read EVB_CHUNK_BLOCK header
    let chunks_blk_size = u32::from_le_bytes(block[0..4].try_into().unwrap()) as usize;
    if chunks_blk_size < 8 || chunks_blk_size > stored_size as usize {
        return Err(Error::VfsParse(format!(
            "invalid chunk block size: {} vs stored {}",
            chunks_blk_size, stored_size
        )));
    }

    // Read chunk table (remaining bytes after the 8-byte header)
    let table_bytes = &block[8..chunks_blk_size];
    let table_u32: Vec<u32> = table_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();

    // Every 3rd entry is the actual chunk size
    let chunk_sizes: Vec<usize> = table_u32
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 == 0)
        .map(|(_, &v)| v as usize)
        .collect();

    if chunk_sizes.is_empty() {
        return Err(Error::VfsParse("empty chunk table".into()));
    }

    // Compressed data starts after the chunk block table
    let compressed_data = &block[chunks_blk_size..];
    let mut compressed_pos = 0;
    let mut output = Vec::with_capacity(original_size as usize);

    for &chunk_size in &chunk_sizes {
        if compressed_pos + chunk_size > compressed_data.len() {
            return Err(Error::UnexpectedEof);
        }
        let chunk = &compressed_data[compressed_pos..compressed_pos + chunk_size];
        compressed_pos += chunk_size;

        let dec = aplib::decompress(chunk, false)
            .map_err(|e| Error::VfsParse(format!("aplib: {e}")))?;
        output.extend_from_slice(&dec);
    }

    if output.len() != original_size as usize {
        return Err(Error::VfsParse(format!(
            "decompressed size mismatch: expected {} bytes, got {}",
            original_size,
            output.len()
        )));
    }

    Ok(output)
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidPe,
    NoEnigmaSection(String),
    InvalidMagic,
    UnexpectedEof,
    VfsParse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::InvalidPe => write!(f, "Not a valid PE file"),
            Self::NoEnigmaSection(s) => write!(f, "PE section {s} not found"),
            Self::InvalidMagic => write!(f, "Invalid EVB magic: expected 'EVB\\0'"),
            Self::UnexpectedEof => write!(f, "Unexpected end of data"),
            Self::VfsParse(s) => write!(f, "VFS parse error: {s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
