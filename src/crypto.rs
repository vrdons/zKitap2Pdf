use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use block_padding::Pkcs7;
use blowfish::cipher::{BlockDecryptMut, KeyInit};
use blowfish::Blowfish;
use ecb::Decryptor;
use swf::avm2::types::AbcFile;
use swf::{avm2::read::Reader, parse_swf, SwfBuf};

const PUSHSTRING_OPCODE: u8 = 44;
const STRING_PREFIX: &str = "==";
const DECRYPT_KEY: &str = "pub1isher1l0O";

/// Extracts the first decryptable string embedded in an SWF buffer.
///
/// Parses the SWF data, inspects DoAbc2 tags and their method bodies, and returns the
/// first string that can be successfully decrypted by the module's decryption pipeline.
/// Errors encountered during parsing or decryption are propagated.
///
/// # Returns
///
/// `Some(String)` with the decrypted value if a decryptable string is found, `None` otherwise.
///
/// # Examples
///
/// ```no_run
/// use crate::check_process;
/// // `buf` should be a valid SWF buffer (SwfBuf)
/// let buf = /* obtain SwfBuf */ unimplemented!();
/// match check_process(&buf) {
///     Ok(Some(decrypted)) => println!("found: {}", decrypted),
///     Ok(None) => println!("no decryptable strings found"),
///     Err(e) => eprintln!("error: {}", e),
/// }
/// ```
pub fn check_process(buf: &SwfBuf) -> Result<Option<String>> {
    let parsed = parse_swf(buf)?;

    for tag in &parsed.tags {
        let swf::Tag::DoAbc2(tag) = tag else { continue };

        let mut reader = Reader::new(tag.data);
        let abc = reader.read()?;

        for body in &abc.method_bodies {
            if let Some(result) = scan_method_body(&abc, &body.code)? {
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
}

/// Finds the first constant string pushed in an ABC method body that begins with `STRING_PREFIX` and returns its decrypted contents.
///
/// The function scans the provided bytecode for PUSHSTRING opcodes, resolves the referenced constant-pool string, and, if the string starts with `STRING_PREFIX`, decrypts it using `KKDecryptor` and `DECRYPT_KEY`. If no matching pushed string is found, `None` is returned.
///
/// # Returns
///
/// `Some(String)` with the decrypted text if a matching pushed constant string is found and successfully decrypted, `None` otherwise.
///
/// # Examples
///
/// ```no_run
/// // Given an `AbcFile` `abc` and a method body byte slice `code`,
/// // attempt to locate and decrypt the first matching pushed string:
/// let result = scan_method_body(&abc, code)?;
/// if let Some(decrypted) = result {
///     println!("decrypted: {}", decrypted);
/// }
/// ```
fn scan_method_body(abc: &AbcFile, code: &[u8]) -> Result<Option<String>> {
    let mut pc = 0usize;
    let decryptor = KKDecryptor;

    while pc < code.len() {
        let opcode = code[pc];
        pc += 1;

        if opcode != PUSHSTRING_OPCODE {
            continue;
        }

        let idx = read_u30(code, &mut pc);

        let Some(raw) = abc.constant_pool.strings.get(idx) else {
            continue;
        };

        let Ok(text) = std::str::from_utf8(raw) else {
            continue;
        };

        if text.starts_with(STRING_PREFIX) {
            return decryptor
                .decrypt(text, DECRYPT_KEY)
                .map(Some)
                .map_err(|e| anyhow!(e));
        }
    }

    Ok(None)
}

/// Decode a variable-length unsigned 30-bit integer from a byte slice, advancing the program counter.
///
/// Reads a u30 encoded with 7-bit payload bytes and a continuation bit (0x80) per byte,
/// starting at `*pc`. Advances `*pc` past the bytes consumed by the integer.
///
/// # Parameters
///
/// - `code`: source byte slice containing the encoded integer.
/// - `pc`: index into `code` where decoding starts; will be incremented to point just after the decoded value.
///
/// # Returns
///
/// The decoded `usize` value.
///
/// # Examples
///
/// ```
/// let bytes = [0x85, 0x01]; // 0x85 -> lower 7 bits 0x05 with continuation, 0x01 -> 0x01 => value = 0x85 & 0x7F | ((0x01 & 0x7F) << 7) = 0x85
/// let mut pc = 0usize;
/// let v = read_u30(&bytes, &mut pc);
/// assert_eq!(v, 0x85);
/// assert_eq!(pc, 2);
/// ```
fn read_u30(code: &[u8], pc: &mut usize) -> usize {
    let mut result = 0usize;
    let mut shift = 0;

    loop {
        let b = code[*pc];
        *pc += 1;

        result |= ((b & 0x7F) as usize) << shift;

        if b & 0x80 == 0 {
            break;
        }

        shift += 7;
    }

    result
}

// ============================================================

pub struct KKDecryptor;

impl KKDecryptor {
    /// Decrypts an encoded string using the module's FD2 transformation followed by Blowfish-ECB/PKCS#7 decryption.
    ///
    /// Applies the FD2 transformation to `data` using `key`, removes whitespace, base64-decodes the result,
    /// decrypts the decoded bytes with Blowfish in ECB mode using `key` and PKCS#7 padding, and returns the decrypted
    /// bytes as a UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns an error if any step of the transformation, base64 decoding, cipher construction, or decryption fails.
    ///
    /// # Examples
    ///
    /// ```
    /// let decryptor = KKDecryptor {};
    /// let encoded = "==..."; // an example encoded string produced by the corresponding encoder
    /// let key = "pub1isher1l0O";
    /// let plain = decryptor.decrypt(encoded, key).unwrap();
    /// println!("{}", plain);
    /// ```
    pub fn decrypt(&self, data: &str, key: &str) -> Result<String> {
        let transformed = self.fd2_transform(data, key)?;

        let cleaned: String = transformed.chars().filter(|c| !c.is_whitespace()).collect();

        let mut encrypted = BASE64
            .decode(cleaned)?;

        type BlowfishEcb = Decryptor<Blowfish>;

        let cipher = BlowfishEcb::new_from_slice(key.as_bytes())?;

        let decrypted = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut encrypted).unwrap();

        Ok(String::from_utf8_lossy(decrypted).into_owned())
    }

    /// Perform the FD2 preprocessing pipeline on `input`.
    ///
    /// The input string is reversed, all whitespace removed, base64-decoded, and then XORed
    /// against `key` using a repeating-key XOR. The resulting bytes are converted to a UTF-8
    /// string; invalid sequences are replaced (lossy).
    ///
    /// # Examples
    ///
    /// ```
    /// let dec = crate::crypto::KKDecryptor;
    /// let out = dec.fd2_transform("=...base64...", "secret").unwrap();
    /// assert!(out.len() > 0);
    /// ```
    fn fd2_transform(&self, input: &str, key: &str) -> Result<String> {
        let reversed: String = input.chars().rev().collect();
        let cleaned: String = reversed.chars().filter(|c| !c.is_whitespace()).collect();

        let decoded = BASE64
            .decode(cleaned)?;

        let xored = self.apply_xor(&decoded, key);
        Ok(String::from_utf8_lossy(&xored).into_owned())
    }

    /// Applies a repeating-key XOR between `data` and `key`.
    ///
    /// The key is used cyclically: each byte of `data` is XORed with the corresponding byte
    /// from `key`, wrapping to the start of `key` when its length is exceeded.
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the XORed bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// let decryptor = super::KKDecryptor;
    /// let data = b"hello";
    /// let out = decryptor.apply_xor(data, "k");
    /// // each byte XORed with 'k' (0x6B)
    /// assert_eq!(out, data.iter().map(|&b| b ^ 0x6B).collect::<Vec<u8>>());
    /// ```
    fn apply_xor(&self, data: &[u8], key: &str) -> Vec<u8> {
        let key = key.as_bytes();
        let key_len = key.len();

        data.iter()
            .enumerate()
            .map(|(i, &b)| b ^ key[i % key_len])
            .collect()
    }
}