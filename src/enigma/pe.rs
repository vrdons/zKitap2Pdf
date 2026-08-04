//! PE32/PE32+ parser — just enough to read DOS stub, section table, and
//! extract raw section data.

use crate::enigma::Error;

#[derive(Debug, Clone)]
pub struct PeInfo {
    /// 64-bit (PE32+) vs 32-bit (PE32) image. Kept for future use (e.g. handling
    /// 64-bit Fernus projectors) even though the current pipeline ignores it.
    #[allow(dead_code)]
    pub is_64bit: bool,
    pub sections: Vec<SectionInfo>,
}

#[derive(Debug, Clone)]
pub struct SectionInfo {
    pub name: String,
    /// Virtual layout. Not consumed by the unpacker today but part of the PE
    /// info surface; retained so callers can map RVA → file offsets if needed.
    #[allow(dead_code)]
    pub virtual_address: u32,
    #[allow(dead_code)]
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
}

impl PeInfo {
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        if raw.len() < 64 {
            return Err(Error::InvalidPe);
        }

        if &raw[0..2] != b"MZ" {
            return Err(Error::InvalidPe);
        }

        let pe_offset = u32::from_le_bytes(raw[0x3c..0x40].try_into().unwrap()) as usize;
        if pe_offset + 4 > raw.len() {
            return Err(Error::InvalidPe);
        }

        if &raw[pe_offset..pe_offset + 4] != b"PE\0\0" {
            return Err(Error::InvalidPe);
        }

        let coff_header_offset = pe_offset + 4;

        let num_sections = u16::from_le_bytes(
            raw[coff_header_offset + 2..coff_header_offset + 4]
                .try_into()
                .unwrap(),
        );

        let opt_header_size = u16::from_le_bytes(
            raw[coff_header_offset + 16..coff_header_offset + 18]
                .try_into()
                .unwrap(),
        );

        let opt_header_start = coff_header_offset + 20;

        let magic = u16::from_le_bytes(
            raw[opt_header_start..opt_header_start + 2]
                .try_into()
                .unwrap(),
        );

        let is_64bit = magic == 0x20b;

        let section_table_start = opt_header_start + opt_header_size as usize;

        let mut sections = Vec::new();
        for i in 0..num_sections as usize {
            let sec_off = section_table_start + i * 40;
            if sec_off + 40 > raw.len() {
                break;
            }

            let name_bytes = &raw[sec_off..sec_off + 8];
            let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_string();

            let virtual_size =
                u32::from_le_bytes(raw[sec_off + 8..sec_off + 12].try_into().unwrap());
            let virtual_address =
                u32::from_le_bytes(raw[sec_off + 12..sec_off + 16].try_into().unwrap());
            let raw_size = u32::from_le_bytes(raw[sec_off + 16..sec_off + 20].try_into().unwrap());
            let raw_offset =
                u32::from_le_bytes(raw[sec_off + 20..sec_off + 24].try_into().unwrap());

            sections.push(SectionInfo {
                name,
                virtual_address,
                virtual_size,
                raw_offset,
                raw_size,
            });
        }

        Ok(PeInfo { is_64bit, sections })
    }

    /// Look up a section's raw bytes by name. Reserved for diagnostics / future
    /// section-driven detection; the VFS unpacker reads sections directly.
    #[allow(dead_code)]
    pub fn section_data<'a>(&self, raw: &'a [u8], name: &str) -> Option<&'a [u8]> {
        self.sections.iter().find(|s| s.name == name).and_then(|s| {
            let start = s.raw_offset as usize;
            let end = (s.raw_offset + s.raw_size) as usize;
            if end <= raw.len() {
                Some(&raw[start..end])
            } else {
                None
            }
        })
    }
}
