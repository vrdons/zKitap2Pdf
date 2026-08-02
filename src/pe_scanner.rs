//! PE32 EXE scanner for embedded SWF files and ABC string extraction.

use anyhow::{Result, anyhow};
use flate2::read::ZlibDecoder;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

/// A validated SWF entry found embedded in a binary.
#[derive(Debug, Clone)]
pub struct SwfEntry {
    pub offset: u64,
    pub signature: &'static str,
    pub version: u8,
    pub declared_length: u32,
    pub classification: &'static str,
    pub frame_count: u16,
    pub raw_bytes: Vec<u8>,
}

/// Extract all ABC string constants (from DoABC/DoABC2 tags) across all SWFs
/// in the EXE. Useful for finding hardcoded keys like `PUBLISHER_KEY`.
pub fn extract_abc_strings(raw_exe: &[u8]) -> Result<HashSet<String>> {
    let scanner = ExeScanner::from_bytes(raw_exe.to_vec());
    let swfs = scanner.scan_swfs()?;
    let mut strings = HashSet::new();

    for swf in &swfs {
        for s in extract_strings_from_fws(&swf.raw_bytes) {
            strings.insert(s);
        }
    }
    Ok(strings)
}

/// Find a string in the ABC pool that contains `substring`.
pub fn find_abc_string_containing(raw_exe: &[u8], substring: &str) -> Result<Option<String>> {
    let strings = extract_abc_strings(raw_exe)?;
    let lowered = substring.to_lowercase();
    for s in &strings {
        if s.to_lowercase().contains(&lowered) {
            return Ok(Some(s.clone()));
        }
    }
    Ok(None)
}

/// Scanner for Fernus Z-Kitap PE32 executables.
pub struct ExeScanner {
    data: Vec<u8>,
}

impl ExeScanner {
    pub fn from_file(path: &Path) -> Result<Self> {
        Ok(Self {
            data: std::fs::read(path)?,
        })
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Scan the EXE for all embedded SWF files.
    pub fn scan_swfs(&self) -> Result<Vec<SwfEntry>> {
        let data = &self.data;
        let mut swfs = Vec::new();
        if data.len() < 8 {
            return Ok(swfs);
        }

        let mut i = 0;
        let last = data.len() - 8;
        while i <= last {
            let sig_str = match &data[i..i + 3] {
                b"FWS" => "FWS",
                b"CWS" => "CWS",
                b"ZWS" => "ZWS",
                _ => {
                    i += 1;
                    continue;
                }
            };

            let version = data[i + 3];
            if !(1..=50).contains(&version) {
                i += 1;
                continue;
            }

            let declared_length =
                u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]);
            if !(100..=200_000_000).contains(&declared_length) {
                i += 1;
                continue;
            }

            let end = (i as u64 + declared_length as u64).min(data.len() as u64) as usize;
            let nbits = data[i + 8] >> 3;
            if !(4..=19).contains(&nbits) {
                i += 1;
                continue;
            }

            let header = &data[i..end];
            let (fws_bytes, frame_count) = match self.decompress_and_read_frames(header) {
                Ok(v) => v,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };

            let classification = classify_swf(frame_count, header.len());

            if swfs.iter().any(|s: &SwfEntry| s.offset == i as u64) {
                i += 1;
                continue;
            }

            swfs.push(SwfEntry {
                offset: i as u64,
                signature: sig_str,
                version,
                declared_length,
                classification,
                frame_count,
                raw_bytes: fws_bytes,
            });
            // Skip past this SWF body to avoid re-detecting nested signatures.
            i = end;
        }

        Ok(swfs)
    }

    /// Decompress a CWS/ZWS SWF to FWS and read frame count.
    fn decompress_and_read_frames(&self, data: &[u8]) -> Result<(Vec<u8>, u16)> {
        if data.len() < 3 {
            return Ok((data.to_vec(), 0));
        }

        let fws = match data[..3] {
            [b'C', b'W', b'S'] | [b'c', b'w', b's'] if data.len() > 8 => {
                let mut decompressed = Vec::new();
                let mut decoder = ZlibDecoder::new(&data[8..]);
                match decoder.read_to_end(&mut decompressed) {
                    Ok(_) => {
                        let mut result = Vec::with_capacity(8 + decompressed.len());
                        result.extend_from_slice(b"FWS");
                        result.extend_from_slice(&data[3..8]);
                        result.extend_from_slice(&decompressed);
                        result
                    }
                    Err(_) => return Err(anyhow!("zlib failed (false signature)")),
                }
            }
            [b'Z', b'W', b'S'] | [b'z', b'w', b's'] => {
                return Err(anyhow!("ZWS/LZMA not supported"));
            }
            _ => data.to_vec(),
        };

        let frame_count = read_frame_count(&fws);
        Ok((fws, frame_count))
    }
}

/// Read the frame count from a decompressed FWS SWF.
fn read_frame_count(data: &[u8]) -> u16 {
    if data.len() < 12 {
        return 0;
    }
    let nbits = data[8] >> 3;
    if !(4..=19).contains(&nbits) {
        return 0;
    }
    // RECT = 5 bits nbits + 4 fields of nbits = 5 + 4*nbits bits.
    let rect_bytes = (5 + nbits * 4).div_ceil(8) as usize;
    let offset = 8 + rect_bytes;
    if data.len() < offset + 4 {
        return 0;
    }
    // Skip frame_rate (2 bytes), read frame_count (2 bytes).
    u16::from_le_bytes([data[offset + 2], data[offset + 3]])
}

/// Classify an SWF based on frame count and content size.
fn classify_swf(frame_count: u16, len: usize) -> &'static str {
    if frame_count > 1000 {
        return "page";
    }
    if frame_count > 100 && len > 150_000 {
        return "loader";
    }
    if frame_count > 100 {
        return "page";
    }
    "ui"
}

/// Check if a PE32 file is UPX-packed by scanning section names.
pub fn is_upx_packed(data: &[u8]) -> bool {
    let pe_offset = pe_offset(data) as usize;
    if pe_offset == 0 || pe_offset + 4 > data.len() {
        return false;
    }
    if &data[pe_offset..pe_offset + 4] != b"PE\x00\x00" {
        return false;
    }
    // PE32: sizeof(IMAGE_FILE_HEADER) + sizeof(IMAGE_OPTIONAL_HEADER32) = 0xF8.
    let sections_start = pe_offset + 0xF8;
    for i in 0..3 {
        let off = sections_start + i * 40;
        if off + 8 > data.len() {
            break;
        }
        let name = std::str::from_utf8(&data[off..off + 8]).unwrap_or("");
        if name.trim_matches('\0').starts_with("UPX") {
            return true;
        }
    }
    false
}

/// Walk SWF tags and extract string constants from all DoABC (72) and
/// DoABC2 (82) tags. Handles both FWS and CWS input transparently.
pub fn extract_strings_from_fws(fws: &[u8]) -> Vec<String> {
    let mut out = Vec::new();

    // Decompress if CWS → FWS, otherwise use as-is
    let owned: Vec<u8>;
    let data = if fws.len() > 3 && &fws[..3] == b"CWS" && fws.len() > 8 {
        let mut body = Vec::new();
        if ZlibDecoder::new(&fws[8..]).read_to_end(&mut body).is_ok() {
            owned = body;
            &owned[..]
        } else {
            fws
        }
    } else {
        fws
    };

    if data.len() < 8 {
        return out;
    }

    let nbits = data[8] >> 3;
    if !(4..=19).contains(&nbits) {
        return out;
    }
    let rect_bytes = (5 + (nbits as usize) * 4 + 7) / 8;
    let mut pos = 8 + rect_bytes + 4; // skip header + RECT + frame_rate + frame_count

    while pos + 2 <= data.len() {
        let tag_raw = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let tag_code = (tag_raw >> 6) as usize;
        let mut tag_len = (tag_raw & 0x3F) as usize;
        pos += 2;
        if tag_len == 0x3F {
            if pos + 4 > data.len() {
                break;
            }
            tag_len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
            pos += 4;
        }
        if pos + tag_len > data.len() {
            break;
        }

        if tag_code == 72 || tag_code == 82 {
            // DoABC or DoABC2: flags(4) + name(null-term) + ABC bytecode
            let tag_data = &data[pos..pos + tag_len];
            if let Some(name_end) = tag_data.iter().position(|&b| b == 0) {
                let abc_bytes = &tag_data[name_end + 1..];
                if let Some(strings) = parse_abc_string_pool(abc_bytes) {
                    out.extend(strings);
                }
            }
        }

        pos += tag_len;
    }

    out
}

/// Parse an ABC (ActionScript ByteCode) block and return its string
/// constant pool. Returns `None` if the block is malformed.
fn parse_abc_string_pool(abc: &[u8]) -> Option<Vec<String>> {
    if abc.len() < 4 {
        return None;
    }

    let mut pos = 4usize; // skip minor_ver(2) + major_ver(2)

    // Helper: read u30 (variable-length unsigned integer)
    let read_u30 = |buf: &[u8], p: &mut usize| -> Option<u32> {
        let mut val: u32 = 0;
        let mut shift = 0u32;
        loop {
            if *p >= buf.len() {
                return None;
            }
            let b = buf[*p];
            *p += 1;
            val |= ((b & 0x7F) as u32) << shift;
            shift += 7;
            if (b & 0x80) == 0 {
                break;
            }
            if shift > 28 {
                return None; // overflow
            }
        }
        Some(val)
    };

    // Skip int, uint, double pools
    for _ in 0..3 {
        let count = read_u30(abc, &mut pos)?;
        if count > 0 {
            pos = pos.checked_add((count - 1) as usize)?;
        }
    }

    // String pool
    let str_count = read_u30(abc, &mut pos)? as usize;
    if str_count == 0 {
        return Some(Vec::new());
    }
    if str_count > 10_000_000 {
        return None; // sanity
    }

    let mut strings = Vec::with_capacity(str_count.saturating_sub(1));
    for _ in 1..str_count {
        let s_len = read_u30(abc, &mut pos)? as usize;
        if pos + s_len > abc.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&abc[pos..pos + s_len]).into_owned();
        pos += s_len;
        strings.push(s);
    }

    Some(strings)
}

fn pe_offset(data: &[u8]) -> u32 {
    if data.len() < 0x40 {
        return 0;
    }
    u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]])
}
