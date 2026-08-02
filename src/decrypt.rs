//! Fernus Z-Kitap crypto: KK (Blowfish-ECB + reverse-Base64-XOR) and KrySWFCrypto.

use anyhow::{Result, anyhow};
use base64::Engine;
use blowfish::Blowfish;
use blowfish::cipher::{Block, BlockCipherDecrypt, KeyInit};

/// Default publisher key — used only as fallback if extraction from SWF fails.
pub const DEFAULT_PUBLISHER_KEY: &str = "pub1isher1l0O";
pub const DEFAULT_FERNUS_KEY: &str = "kxk";

pub struct KK;

impl KK {
    /// fd2 (reverse+b64+XOR) if requested, then lenient base64 decode + Blowfish-ECB.
    pub fn fd1(data: &str, key_str: &str, apply_fd2: bool) -> Result<String> {
        let decoded: std::borrow::Cow<'_, str> = if apply_fd2 {
            Self::fd2(data, key_str)?.into()
        } else {
            data.into()
        };

        // AS3 Base64Decoder silently skips non-alphabet chars.
        let mut filtered: Vec<u8> = Vec::with_capacity(decoded.len());
        for &b in decoded.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') {
                filtered.push(b);
            }
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&filtered)
            .map_err(|e| anyhow!("fd1 base64: {e}"))?;

        // Blowfish key = hex(ascii(key_str)) interpreted as raw bytes.
        let key_str_bytes = key_str.as_bytes();
        let mut key_bytes = Vec::with_capacity(key_str_bytes.len());
        for &b in key_str_bytes {
            key_bytes.push(b >> 4);
            key_bytes.push(b & 0x0F);
        }

        let decrypted = Self::blowfish_ecb_decrypt(&bytes, &key_bytes)?;
        String::from_utf8(decrypted).map_err(|e| anyhow!("fd1 utf8: {e}"))
    }

    /// Reverse (by byte) + base64 decode + repeating-key XOR.
    pub fn fd2(input: &str, key: &str) -> Result<String> {
        let bytes = input.as_bytes();
        let mut reversed: Vec<u8> = Vec::with_capacity(bytes.len());
        reversed.extend(bytes.iter().rev());

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&reversed)
            .map_err(|e| anyhow!("fd2 base64: {e}"))?;

        let xored = Self::apply_xor(&decoded, key);
        String::from_utf8(xored).map_err(|e| anyhow!("fd2 utf8: {e}"))
    }

    pub fn apply_xor(input: &[u8], key: &str) -> Vec<u8> {
        let key_bytes = key.as_bytes();
        if key_bytes.is_empty() {
            return input.to_vec();
        }
        input
            .iter()
            .zip((0..).map(|i| key_bytes[i % key_bytes.len()]))
            .map(|(&b, k)| b ^ k)
            .collect()
    }

    fn blowfish_ecb_decrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow!("empty Blowfish key"));
        }
        if !data.len().is_multiple_of(8) {
            return Err(anyhow!("ciphertext len {} not multiple of 8", data.len()));
        }
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let bf = <Blowfish as KeyInit>::new_from_slice(key)
            .map_err(|e| anyhow!("invalid Blowfish key: {e:?}"))?;

        let mut result = data.to_vec();
        for chunk in result.chunks_exact_mut(8) {
            let block = <&mut Block<Blowfish>>::try_from(chunk)?;
            bf.decrypt_block(block);
        }

        let pad_len = *result.last().unwrap() as usize;
        if pad_len == 0 || pad_len > 8 {
            return Err(anyhow!("invalid PKCS5 padding length: {pad_len}"));
        }
        let start = result.len() - pad_len;
        if result[start..].iter().any(|&b| b as usize != pad_len) {
            return Err(anyhow!("invalid PKCS5 padding"));
        }
        result.truncate(start);
        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KryCode {
    pub f1: i32,
    pub f2: i32,
    pub f3: i32,
}

/// Verified KryCode for the default Fernus publisher distribution.
pub const DEFAULT_KRY_CODE: KryCode = KryCode {
    f1: 33,
    f2: 20,
    f3: 10,
};

pub struct KrySWFCrypto;

impl KrySWFCrypto {
    /// Decrypt SWF bytes in place. All arithmetic wraps modulo 256.
    pub fn decrypt(bytes: &mut [u8], code: &KryCode) -> Result<()> {
        Self::separate_bytes(bytes, 10000, 11000, code.f1, code.f2);
        Self::separate_bytes(bytes, 5000, 5500, code.f3, code.f1);
        Self::separate_bytes(bytes, 850, 1500, code.f2, code.f3);
        Self::separate_bytes(bytes, 0, 300, code.f1, code.f2);

        let f1 = code.f1 as u8;
        let f2 = code.f2 as u8;
        let f3 = code.f3 as u8;
        let fix = [
            (code.f3 as usize, f3),
            (code.f2 as usize, f2),
            (code.f1 as usize, f1),
            (2, f3),
            (1, f2),
            (0, f1),
        ];
        for (pos, val) in fix {
            if let Some(b) = bytes.get_mut(pos) {
                *b = b.wrapping_sub(val);
            }
        }
        Ok(())
    }

    fn separate_bytes(bytes: &mut [u8], s_index: i32, e_index: i32, n1: i32, n2: i32) {
        let end = ((e_index + n1 * 3) as usize).min(bytes.len());
        let start = (s_index.max(0) as usize).min(bytes.len());
        if start >= end {
            return;
        }

        // In place: subtract n2, reverse, subtract n2 again — equivalent to the
        // original two-pass form but avoids a temporary Vec allocation.
        let slice = &mut bytes[start..end];
        let n2 = n2 as u8;
        for b in slice.iter_mut() {
            *b = b.wrapping_sub(n2);
        }
        slice.reverse();
        for b in slice.iter_mut() {
            *b = b.wrapping_sub(n2);
        }
    }
}

/// Parse fernusCode "XxYxZ" or "X_Y_Z" into a KryCode, adding pkxkname length to each part.
pub fn parse_kry_code(fernus_code_decrypted: &str, pkxkname_len: usize) -> Result<KryCode> {
    let sep = if fernus_code_decrypted.contains('x') {
        'x'
    } else if fernus_code_decrypted.contains('_') {
        '_'
    } else {
        return Err(anyhow!("cannot parse fernusCode: {fernus_code_decrypted}"));
    };

    let mut parts = fernus_code_decrypted.split(sep);
    let mut parse_part = || -> Result<i32> {
        let s = parts
            .next()
            .ok_or_else(|| anyhow!("fernusCode needs 3 parts"))?;
        s.trim()
            .parse::<i32>()
            .map_err(|_| anyhow!("invalid fernusCode part: {s}"))
    };
    let f1 = parse_part()?;
    let f2 = parse_part()?;
    let f3 = parse_part()?;
    if parts.next().is_some() {
        return Err(anyhow!("fernusCode should have exactly 3 parts"));
    }

    let pkxk = pkxkname_len as i32;
    Ok(KryCode {
        f1: f1 + pkxk,
        f2: f2 + pkxk,
        f3: f3 + pkxk,
    })
}
