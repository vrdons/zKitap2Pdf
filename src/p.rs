use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use block_padding::Pkcs7;
use blowfish::Blowfish;
use blowfish::cipher::{BlockDecryptMut, KeyInit};
use ecb::Decryptor;
use serde::{Deserialize, Serialize};
use swf::avm2::types::Op;
use swf::extensions::ReadSwfExt;
use swf::{SwfBuf, avm2::read::Reader, parse_swf};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub fernus_code: String,
    pub color: u32,
    pub server: String,
    pub password_status: bool,
    pub pkxkname: String,
    pub publisher: String,
    pub app_type: String,
    pub server_status: bool,
}

const DECRYPT_KEY: &str = "pub1isher1l0O";

pub fn check_process(buf: &SwfBuf) -> Result<Option<Config>> {
    let parsed = parse_swf(buf)?;

    for tag in &parsed.tags {
        let swf::Tag::DoAbc2(tag) = tag else { continue };

        let mut reader = Reader::new(tag.data);
        let abc = reader.read()?;

        for body in &abc.method_bodies {
            let mut r = Reader::new(&body.code);

            while !r.as_slice().is_empty() {
                let op = r.read_op()?;

                if let Op::PushString { value } = op {
                    let raw = &abc.constant_pool.strings[value.0 as usize];

                    if raw.len() > 50 {
                        if let Ok(s) = std::str::from_utf8(raw) {
                            let decrypted = KKDecryptor
                                .decrypt(s, DECRYPT_KEY)
                                .map_err(|e| anyhow!(e))?;
                            let config: Config = serde_json::from_str(&decrypted)?;
                            return Ok(Some(config));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
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

        let decrypted = cipher
            .decrypt_padded_mut::<Pkcs7>(&mut encrypted)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {:?}", e))?;
        String::from_utf8(decrypted.to_vec())
            .map_err(|e| anyhow::anyhow!("Decrypted data is not valid UTF-8: {}", e))
    }

    fn fd2_transform(&self, input: &str, key: &str) -> Result<String> {
        let reversed: String = input.chars().rev().collect();
        let cleaned: String = reversed.chars().filter(|c| !c.is_whitespace()).collect();

        let decoded = BASE64.decode(cleaned)?;

        let xored = self.apply_xor(&decoded, key);
        String::from_utf8(xored)
            .map_err(|e| anyhow::anyhow!("XOR transform produced invalid UTF-8: {}", e))
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

#[derive(Debug, Default)]
pub struct KkObject {
    pub f1: i32,
    pub f2: i32,
    pub f3: i32,
}
pub fn get_kkobject(config: Config) -> Result<KkObject> {
    let decrypted = KKDecryptor.decrypt(&config.fernus_code, DECRYPT_KEY)?;
    let parts: Vec<i32> = decrypted
        .split('x')
        .map(|p| p.parse::<i32>())
        .collect::<Result<_, _>>()?;

    if parts.len() != 3 {
        anyhow::bail!("fd1 output format invalid: {}", decrypted);
    }
    let len = config.pkxkname.len() as i32;

    Ok(KkObject {
        f1: parts[0] + len,
        f2: parts[1] + len,
        f3: parts[2] + len,
    })
}
