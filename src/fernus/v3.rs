//! v3 Fernus Z-Kitap (Flutter + Enigma Virtual Box) crypto.
//!
//! Handles:
//! - keystr extraction from `kernel_blob.bin` via Dart envied XOR
//! - IV extraction from `kernel_blob.bin`
//! - AES-256-CBC decryption (matching Dart `package:encrypt`)
//! - `createKey` (key derivation matching Dart implementation)

use aes::Aes256;
use base64::Engine;

pub use crate::error::V3Error;

/// Extract keystr from kernel_blob.bin by XORing `_enviedkey_keyStr` and
/// `_envieddata_keyStr` Dart arrays. Each XOR result's low byte is one
/// character of the key string.
pub fn extract_keystr(kernel_data: &[u8]) -> Result<String, V3Error> {
    let key_ints = parse_envied_array(kernel_data, "key")
        .map_err(|e| V3Error::ArrayNotFound(format!("key: {e}")))?;
    let data_ints = parse_envied_array(kernel_data, "data")
        .map_err(|e| V3Error::ArrayNotFound(format!("data: {e}")))?;

    if key_ints.len() != data_ints.len() {
        // Not reachable from parse_envied_array's API but kept for robustness.
        return Err(V3Error::ArrayMismatch {
            tag: ' ',
            expected: "key/data length",
        });
    }

    let chars: String = key_ints
        .iter()
        .zip(data_ints.iter())
        .map(|(&k, &d)| ((k ^ d) & 0xFF) as u8 as char)
        .filter(|&c| c != '\0')
        .collect();

    Ok(chars)
}

/// Extract IV from kernel_blob.bin: `static final iv = IV.fromUtf8('...')`.
pub fn extract_iv(kernel_data: &[u8]) -> Result<String, V3Error> {
    // Search for known patterns
    let patterns: &[&[u8]] = &[
        b"static final iv = IV.fromUtf8('",
        b"static const iv = IV.fromUtf8('",
    ];

    for pat in patterns {
        if let Some(idx) = kernel_data.windows(pat.len()).position(|w| w == *pat) {
            let start = idx + pat.len();
            if let Some(end) = kernel_data[start..].iter().position(|&b| b == b'\'') {
                let iv_bytes = &kernel_data[start..start + end];
                return Ok(std::str::from_utf8(iv_bytes)?.to_string());
            }
        }
    }

    // Fallback: regex-like search for iv = IV.fromUtf8('...')
    let re = regex::bytes::Regex::new(r"iv\s*=\s*IV\.fromUtf8\('([^']+)'")?;
    if let Some(caps) = re.captures(kernel_data)
        && let Some(m) = caps.get(1)
    {
        return Ok(std::str::from_utf8(m.as_bytes())?.to_string());
    }

    Err(V3Error::IvNotFound)
}

/// Dart: `createKey(String key) => key + keystr.substring(0, 32 - key.length)`
pub fn create_key(key: &str, keystr: &str) -> Vec<u8> {
    let key_bytes = key.as_bytes();
    let keystr_bytes = keystr.as_bytes();

    let mut result = key_bytes.to_vec();
    if result.len() < 32 {
        let needed = 32 - result.len();
        let take = needed.min(keystr_bytes.len());
        result.extend_from_slice(&keystr_bytes[..take]);
    }
    result.truncate(32);
    result
}

/// Dart: `decryptString` — reverse string, base64-decode, AES-CBC decrypt, PKCS7 unpad.
pub fn decrypt_string(encrypted: &str, key: &[u8], iv: &[u8]) -> Result<Vec<u8>, V3Error> {
    let reversed: String = encrypted.chars().rev().collect();

    // Dart Base64Decoder is lenient — try to decode with added padding
    let mut raw = reversed.clone();
    // Add padding if needed
    while !raw.len().is_multiple_of(4) {
        raw.push('=');
    }

    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&raw)
        .map_err(|_| V3Error::ArrayNotFound("decrypt_string: base64 decode".into()))?;

    aes_cbc_decrypt(&ciphertext, key, iv)
}

/// Dart: `decryptByte` — AES-CBC decrypt directly, PKCS7 unpad.
pub fn decrypt_bytes(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, V3Error> {
    aes_cbc_decrypt(data, key, iv)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a Dart envied array like `_enviedkey_keyStr = <int>[1, 2, 3];`
///
/// Values are parsed as `i64` (not `i32`) because the envied XOR obfuscation
/// can produce masked ints that exceed `i32::MAX` (e.g. 3_290_869_934). The
/// low byte of each value (after XOR with its data counterpart) is what
/// actually matters, so the wider type is harmless.
fn parse_envied_array(kernel_data: &[u8], suffix: &str) -> Result<Vec<i64>, V3Error> {
    let needle = format!("_envied{suffix}_keyStr = <int>[");
    let needle_bytes = needle.as_bytes();

    let idx = kernel_data
        .windows(needle_bytes.len())
        .position(|w| w == needle_bytes)
        .ok_or_else(|| V3Error::ArrayNotFound(format!("_envied{suffix}_keyStr")))?;

    let start = idx + needle_bytes.len();
    let end = kernel_data[start..]
        .iter()
        .position(|&b| b == b']')
        .ok_or(V3Error::ArrayUnterminated)?;

    let text = std::str::from_utf8(&kernel_data[start..start + end])?;

    // Parse integers as i64 to handle XOR-masked values > i32::MAX.
    let mut ints = Vec::new();
    for part in text.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val: i64 = trimmed.parse().map_err(V3Error::BadInt)?;
        ints.push(val);
    }

    Ok(ints)
}

/// AES-256-CBC decrypt + PKCS7 unpad.
fn aes_cbc_decrypt(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>, V3Error> {
    if key.len() != 32 {
        return Err(V3Error::AesKeyLen(key.len()));
    }
    if iv.len() != 16 {
        return Err(V3Error::AesIvLen(iv.len()));
    }
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if !data.len().is_multiple_of(16) {
        return Err(V3Error::AesBlockLen(data.len()));
    }

    use aes::cipher::{Block, BlockCipherDecrypt, KeyInit};

    let aes = Aes256::new_from_slice(key).map_err(|_| V3Error::AesKeyLen(key.len()))?;
    let mut prev = iv.to_vec(); // 16 bytes

    let mut buf = data.to_vec();
    for chunk in buf.chunks_exact_mut(16) {
        // Decrypt block
        let mut block = Block::<Aes256>::default();
        block.copy_from_slice(chunk);
        aes.decrypt_block(&mut block);

        // XOR with previous ciphertext (or IV for first block)
        for i in 0..16 {
            block[i] ^= prev[i];
        }

        // Save ciphertext for next block
        prev.copy_from_slice(chunk);

        // Write plaintext
        chunk.copy_from_slice(&block);
    }

    // PKCS7 unpad
    // Safety: `data` was non-empty and length was multiple of 16, so `last` exists.
    let pad_len = *buf.last().unwrap() as usize;
    if pad_len == 0 || pad_len > 16 {
        return Err(V3Error::InvalidPkcs7);
    }
    let start = buf.len() - pad_len;
    if buf[start..].iter().any(|&b| b as usize != pad_len) {
        return Err(V3Error::InvalidPkcs7);
    }
    buf.truncate(start);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_key() {
        let keystr = "1l0O1l0O1l0O1l0O1l0O1l0O1l0O1l0O"; // 32 bytes
        let key = create_key("7_14_22", keystr);
        assert_eq!(key.len(), 32);

        let key = create_key("modelegitim", keystr);
        assert_eq!(key.len(), 32);
        assert_eq!(&key[..11], b"modelegitim");
        assert_eq!(&key[11..32], &keystr.as_bytes()[..21]);
    }
}
