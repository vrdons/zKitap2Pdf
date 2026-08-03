//! Fernus `.frns` file format (v2 / newer encryption).
//!
//! Newer Fernus books (circa 2023+) drop `.frns` files instead of `.dll`
//! 
use std::io::Cursor;
use std::num::NonZeroUsize;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use lzma_rs::decompress::raw::{LzmaDecoder, LzmaParams, LzmaProperties};
use serde::Deserialize;

use super::crypto::{KryCode, KrySWFCrypto};

// ---------------------------------------------------------------------------
// LZMA helpers
// ---------------------------------------------------------------------------

/// Minimum bytes for a Flash LZMA header.
const LZMA_HEADER_SIZE: usize = 13;

/// Decompress a Flash LZMA stream (13-byte header + raw LZMA1 payload).
pub fn decompress_flash_lzma(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < LZMA_HEADER_SIZE {
        bail!(
            "LZMA data too short: {} bytes (need ≥ {LZMA_HEADER_SIZE})",
            data.len()
        );
    }

    let props = data[0];
    let lc = (props % 9) as u32;
    let remaining = props / 9;
    let lp = (remaining % 5) as u32;
    let pb = (remaining / 5) as u32;

    let dict_size =
        u32::from_le_bytes(data[1..5].try_into().unwrap());

    let uncomp_size =
        u64::from_le_bytes(data[5..13].try_into().unwrap());

    let properties = LzmaProperties { lc, lp, pb };
    let params = LzmaParams::new(properties, dict_size, Some(uncomp_size));

    let memlimit = NonZeroUsize::new(128 * 1024 * 1024) // 128 MiB — plenty for SWFs
        .ok_or_else(|| anyhow!("invalid memlimit"))?;

    let mut decoder = LzmaDecoder::new(params, Some(memlimit.get()))
        .map_err(|e| anyhow!("LZMA decoder init: {e}"))?;

    let mut input = Cursor::new(&data[LZMA_HEADER_SIZE..]);
    let mut output = Vec::with_capacity(uncomp_size as usize);
    decoder
        .decompress(&mut input, &mut output)
        .map_err(|e| anyhow!("LZMA decompress: {e}"))?;

    Ok(output)
}

// ---------------------------------------------------------------------------
// JSON structures
// ---------------------------------------------------------------------------

/// A single frame entry inside a `.frns` book JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct FrnsFrame {
    /// Frame number (1-based). Some books ship this as a string (`"1"`), others
    /// as a number (`1`), so we use `serde_json::Value` and coerce it.
    #[serde(deserialize_with = "deser_coerce_u32")]
    pub frame: u32,
    /// Page width in pixels. Integer or float, coerced to `f64`.
    #[serde(deserialize_with = "deser_coerce_f64")]
    pub width: f64,
    /// Page height in pixels. Integer or float, coerced to `f64`.
    #[serde(deserialize_with = "deser_coerce_f64")]
    pub height: f64,
    /// Encrypted SWF payload: `{b64}+/={b64}`.
    pub data: String,
}

fn deser_coerce_u32<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => n.as_u64().map(|x| x as u32)
            .ok_or_else(|| Error::custom("frame not a valid u32")),
        serde_json::Value::String(s) => s.parse::<u32>()
            .map_err(|e| Error::custom(format!("frame string not u32: {e}"))),
        _ => Err(Error::custom("frame must be number or string")),
    }
}

fn deser_coerce_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => n.as_f64()
            .ok_or_else(|| Error::custom("width/height not a valid f64")),
        serde_json::Value::String(s) => s.parse::<f64>()
            .map_err(|e| Error::custom(format!("width/height string not f64: {e}"))),
        _ => Err(Error::custom("width/height must be number or string")),
    }
}

/// Root structure of `sysb.frns` / `sysm.frns`.
#[derive(Debug, Clone, Deserialize)]
pub struct FrnsBook {
    /// Total number of frames in the book.
    #[serde(rename = "totalFrames")]
    pub total_frames: u32,
    /// All frames (sorted by `frame` in the runtime, we keep order as-is).
    pub frames: Vec<FrnsFrame>,
}

// ---------------------------------------------------------------------------
// Frame data decryption
// ---------------------------------------------------------------------------

/// Separator that splits the XOR-encoded part from the encrypted payload.
const FRAME_SEPARATOR: &str = "+/=";

impl FrnsFrame {
    /// Decrypt this frame's `data` payload into raw SWF bytes.
    ///
    /// This mirrors `BookClip.gotoAndStop`:
    ///
    /// ```actionscript
    /// var str = this.data.frames[...].data;
    /// str = this.decode(str.split("+/=")[0], f1+f2+f3) + str.split("+/=")[1];
    /// var byte = Base64.decodeToByteArray(str);
    /// byte = kry.decrypte(byte, kkObject);
    /// ```
    pub fn decrypt(&self, code: &KryCode) -> Result<Vec<u8>> {
        let xor_key = (code.f1 + code.f2 + code.f3).to_string();

        let (part0, part1) = self
            .data
            .split_once(FRAME_SEPARATOR)
            .ok_or_else(|| anyhow!("frame data missing '{FRAME_SEPARATOR}' separator"))?;

        // decode(part0, xor_key) → base64 → XOR → UTF-8
        let decoded_part0 = xor_decode_b64(part0, &xor_key)?;

        // decoded_part0 + part1 → base64 → KrySWFCrypto
        let combined = decoded_part0 + part1;

        // AS3's Base64Decoder is lenient — filter to valid b64 alphabet
        let filtered: String = combined
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
            .collect();

        let mut encrypted = base64::engine::general_purpose::STANDARD
            .decode(&filtered)
            .map_err(|e| anyhow!("frame base64 decode: {e}"))?;

        KrySWFCrypto::decrypt(&mut encrypted, code)?;

        Ok(encrypted)
    }
}

/// Mirror of `BookClip.decode` / `Base64.decodeToByteArray` + `applyXor`.
///
/// ```actionscript
/// var inputBuffer = Base64.decodeToByteArray(input);
/// var out = applyXor(inputBuffer, key);
/// out.position = 0;
/// return out.readUTFBytes(out.length);
/// ```
fn xor_decode_b64(b64: &str, key: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| anyhow!("xor_decode base64: {e}"))?;

    let key_bytes = key.as_bytes();
    let xored: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| b ^ key_bytes[i % key_bytes.len()])
        .collect();

    String::from_utf8(xored).map_err(|e| anyhow!("xor_decode utf8: {e}"))
}

// ---------------------------------------------------------------------------
// High-level book loading
// ---------------------------------------------------------------------------

/// Load a `.frns` book file, decompress its LZMA JSON, and decrypt all frame
/// payloads into a `Vec` of raw SWF byte buffers.
///
/// Returns the decrypted SWF bytes together with the frame metadata (width,
/// height, frame number) so the caller can patch SWF headers and render.
pub fn load_and_decrypt_book(data: &[u8], code: &KryCode) -> Result<Vec<DecryptedFrame>> {
    let json_bytes = decompress_flash_lzma(data).context("decompress frns book")?;
    let json_str = std::str::from_utf8(&json_bytes).context("frns JSON utf8")?;
    let book: FrnsBook =
        serde_json::from_str(json_str).context("parse frns book JSON")?;

    let mut frames = Vec::with_capacity(book.frames.len());
    for f in &book.frames {
        let swf_bytes = f
            .decrypt(code)
            .with_context(|| format!("decrypt frame {}", f.frame))?;
        frames.push(DecryptedFrame {
            frame: f.frame,
            width: f.width,
            height: f.height,
            swf_bytes,
        });
    }
    Ok(frames)
}

/// A single fully-decrypted frame ready for header patching and rendering.
#[derive(Debug)]
pub struct DecryptedFrame {
    pub frame: u32,
    pub width: f64,
    pub height: f64,
    pub swf_bytes: Vec<u8>,
}
