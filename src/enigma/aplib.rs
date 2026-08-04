//! Pure-Rust aPLib decompression (LZ77 variant used by Enigma Virtual Box).
//! Ported from the Python `aplib` package v0.6.

/// Decompress an aPLib-compressed buffer (without AP32 header).
/// `strict` produces errors on checksum mismatch; set to `false` for EVB chunks.
pub fn decompress(data: &[u8], strict: bool) -> Result<Vec<u8>, AplibError> {
    // If data starts with "AP32" header, parse it (though EVB chunks typically don't)
    let payload = if data.len() >= 24 && &data[0..4] == b"AP32" {
        let header_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let packed_size = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        let _packed_crc = u32::from_le_bytes(data[12..16].try_into().unwrap());
        let _orig_size = u32::from_le_bytes(data[16..20].try_into().unwrap());
        let _orig_crc = u32::from_le_bytes(data[20..24].try_into().unwrap());

        if header_size + packed_size > data.len() {
            return Err(AplibError::InvalidHeader);
        }
        if strict {
            let actual_packed = &data[header_size..header_size + packed_size];
            let crc = crc32(actual_packed);
            if crc != _packed_crc {
                return Err(AplibError::PackedCrcMismatch);
            }
        }
        &data[header_size..header_size + packed_size]
    } else {
        data
    };

    let mut dec = AplibDecoder::new(payload);
    let result = dec.depack(strict)?;
    Ok(result)
}

/// Internal aPLib decoder state machine.
struct AplibDecoder<'a> {
    src: &'a [u8],
    src_pos: usize,
    dst: Vec<u8>,
    tag: u8,
    bitcount: i32,
}

impl<'a> AplibDecoder<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            src_pos: 0,
            dst: Vec::new(),
            tag: 0,
            bitcount: 0,
        }
    }

    /// Read one byte from the compressed stream.
    fn read_byte(&mut self) -> Result<u8, AplibError> {
        if self.src_pos >= self.src.len() {
            return Err(AplibError::UnexpectedEof);
        }
        let b = self.src[self.src_pos];
        self.src_pos += 1;
        Ok(b)
    }

    /// Get one bit from the tag byte (LSB first in original implementation,
    /// but this matches the Python version which uses MSB-first).
    fn getbit(&mut self) -> Result<u8, AplibError> {
        self.bitcount -= 1;
        if self.bitcount < 0 {
            self.tag = self.read_byte()?;
            self.bitcount = 7;
        }
        let bit = (self.tag >> 7) & 1;
        self.tag <<= 1;
        Ok(bit)
    }

    /// Read a gamma-coded integer (Elias gamma code).
    fn getgamma(&mut self) -> Result<usize, AplibError> {
        let mut result: usize = 1;
        loop {
            result = (result << 1) + self.getbit()? as usize;
            if self.getbit()? == 0 {
                break;
            }
        }
        Ok(result)
    }

    /// Main decompression loop.
    fn depack(&mut self, _strict: bool) -> Result<Vec<u8>, AplibError> {
        // First byte is literal
        let first = self.read_byte()?;
        self.dst.push(first);

        let mut r0: isize = -1;
        let mut lwm: usize = 0;
        let mut done = false;

        while !done {
            if self.getbit()? != 0 {
                // Match
                if self.getbit()? != 0 {
                    if self.getbit()? != 0 {
                        // Short match (4-bit offset)
                        let mut offs: usize = 0;
                        for _ in 0..4 {
                            offs = (offs << 1) + self.getbit()? as usize;
                        }
                        if offs != 0 {
                            let idx = self.dst.len().wrapping_sub(offs);
                            if idx < self.dst.len() {
                                let b = self.dst[idx];
                                self.dst.push(b);
                            } else {
                                // offs == 0 pushed only when offs is actually 0
                            }
                        } else {
                            self.dst.push(0);
                        }
                        lwm = 0;
                    } else {
                        // Single byte with offset
                        let b = self.read_byte()?;
                        let offs = (b >> 1) as usize;
                        let length = 2 + (b & 1) as usize;
                        if offs != 0 {
                            for _ in 0..length {
                                let idx = self.dst.len().wrapping_sub(offs);
                                if idx < self.dst.len() {
                                    let byte = self.dst[idx];
                                    self.dst.push(byte);
                                } else {
                                    return Err(AplibError::InvalidOffset);
                                }
                            }
                        } else {
                            done = true;
                        }
                        r0 = offs as isize;
                        lwm = 1;
                    }
                } else {
                    // Long match
                    let mut offs = self.getgamma()?;
                    if lwm == 0 && offs == 2 {
                        // Reuse previous offset
                        offs = r0 as usize;
                        let mut length = self.getgamma()?;
                        for _ in 0..length {
                            let idx = self.dst.len().wrapping_sub(offs);
                            if idx < self.dst.len() {
                                let byte = self.dst[idx];
                                self.dst.push(byte);
                            } else {
                                return Err(AplibError::InvalidOffset);
                            }
                        }
                    } else {
                        if lwm == 0 {
                            offs = offs.wrapping_sub(3);
                        } else {
                            offs = offs.wrapping_sub(2);
                        }
                        offs <<= 8;
                        offs += self.read_byte()? as usize;
                        let mut length = self.getgamma()?;
                        // Adjust length based on offset magnitude
                        if offs >= 32000 {
                            length += 1;
                        }
                        if offs >= 1280 {
                            length += 1;
                        }
                        if offs < 128 {
                            length += 2;
                        }
                        for _ in 0..length {
                            let idx = self.dst.len().wrapping_sub(offs);
                            if idx < self.dst.len() {
                                let byte = self.dst[idx];
                                self.dst.push(byte);
                            } else {
                                return Err(AplibError::InvalidOffset);
                            }
                        }
                        r0 = offs as isize;
                    }
                    lwm = 1;
                }
            } else {
                // Literal byte
                let b = self.read_byte()?;
                self.dst.push(b);
                lwm = 0;
            }
        }

        Ok(std::mem::take(&mut self.dst))
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[derive(Debug)]
pub enum AplibError {
    UnexpectedEof,
    InvalidOffset,
    InvalidHeader,
    PackedCrcMismatch,
}

impl std::fmt::Display for AplibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AplibError::UnexpectedEof => write!(f, "unexpected end of aPLib stream"),
            AplibError::InvalidOffset => write!(f, "invalid offset in aPLib stream"),
            AplibError::InvalidHeader => write!(f, "invalid AP32 header"),
            AplibError::PackedCrcMismatch => write!(f, "packed data CRC mismatch"),
        }
    }
}

impl std::error::Error for AplibError {}
