use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use block_padding::Pkcs7;
use blowfish::Blowfish;
use blowfish::cipher::{BlockDecryptMut, KeyInit};
use ecb::Decryptor;
use swf::avm2::types::AbcFile;
use swf::{SwfBuf, avm2::read::Reader, parse_swf};

const PUSHSTRING_OPCODE: u8 = 44;
const STRING_PREFIX: &str = "==";
const DECRYPT_KEY: &str = "pub1isher1l0O";

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

fn read_u30(code: &[u8], pc: &mut usize) -> usize {
    let mut result = 0usize;
    let mut shift = 0;

    loop {
        if *pc >= code.len() {
            break;
        }
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
    pub fn decrypt(&self, data: &str, key: &str) -> Result<String> {
        let transformed = self.fd2_transform(data, key)?;

        let cleaned: String = transformed.chars().filter(|c| !c.is_whitespace()).collect();

        let mut encrypted = BASE64.decode(cleaned)?;

        type BlowfishEcb = Decryptor<Blowfish>;

        let cipher = BlowfishEcb::new_from_slice(key.as_bytes())?;

        let decrypted = cipher.decrypt_padded_mut::<Pkcs7>(&mut encrypted).unwrap();

        Ok(String::from_utf8_lossy(decrypted).into_owned())
    }

    fn fd2_transform(&self, input: &str, key: &str) -> Result<String> {
        let reversed: String = input.chars().rev().collect();
        let cleaned: String = reversed.chars().filter(|c| !c.is_whitespace()).collect();

        let decoded = BASE64.decode(cleaned)?;

        let xored = self.apply_xor(&decoded, key);
        Ok(String::from_utf8_lossy(&xored).into_owned())
    }

    fn apply_xor(&self, data: &[u8], key: &str) -> Vec<u8> {
        let key = key.as_bytes();
        let key_len = key.len();

        data.iter()
            .enumerate()
            .map(|(i, &b)| b ^ key[i % key_len])
            .collect()
    }
}
