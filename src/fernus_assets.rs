//! External Fernus Z-Kitap asset discovery and decryption (wine-free path).

use crate::decrypt::{self, KK, KryCode, KrySWFCrypto, DEFAULT_PUBLISHER_KEY};
use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const PUBLISHER_FILE: &str = "publisher.kxk";
pub const SYSD_FILE: &str = "sysd.dll";
pub const SYSB_FILE: &str = "sysb.dll";
pub const SYSM_FILE: &str = "sysm.dll";

/// Fernus assets discovered next to the projector EXE.
#[derive(Debug, Clone)]
pub struct FernusAssets {
    pub root: PathBuf,
    pub publisher: Option<PathBuf>,
    pub sysd: Option<PathBuf>,
    pub sysb: Option<PathBuf>,
    pub sysm: Option<PathBuf>,
}

impl FernusAssets {
    /// Locate the asset bundle relative to the projector EXE.
    pub fn find(exe_path: &Path) -> Self {
        let parent = exe_path.parent().unwrap_or_else(|| Path::new("."));
        let book_stem = exe_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        let same = parent.to_path_buf();
        let files = parent.join("files");
        let windows_dir = parent.join(format!("{book_stem}-windows"));
        let stem_dir = parent.join(book_stem);

        for dir in [same, files, windows_dir, stem_dir] {
            if dir.join(SYSD_FILE).exists() || dir.join(SYSB_FILE).exists() {
                return Self::from_dir(&dir);
            }
        }
        Self {
            root: parent.to_path_buf(),
            publisher: None,
            sysd: None,
            sysb: None,
            sysm: None,
        }
    }

    pub fn from_dir(dir: &Path) -> Self {
        let lookup = |name: &str| {
            let p = dir.join(name);
            p.exists().then_some(p)
        };
        Self {
            root: dir.to_path_buf(),
            publisher: lookup(PUBLISHER_FILE),
            sysd: lookup(SYSD_FILE),
            sysb: lookup(SYSB_FILE),
            sysm: lookup(SYSM_FILE),
        }
    }

    pub fn has_pages(&self) -> bool {
        self.sysb.is_some()
    }
}

/// Parsed publisher configuration (`publisher.kxk` JSON).
#[derive(Debug, Clone, Deserialize)]
pub struct PublisherConfig {
    #[serde(default)]
    pub pkxkname: Option<String>,
    #[serde(default)]
    pub fernus_code: Option<String>,
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, serde_json::Value>,
}

impl PublisherConfig {
    pub fn from_file(path: &Path, publisher_key: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("reading {}: {}", path.display(), e))?;
        Self::parse_with_key(&raw, publisher_key)
    }

    pub fn from_file_default(path: &Path) -> Result<Self> {
        Self::from_file(path, DEFAULT_PUBLISHER_KEY)
    }

    fn parse_with_key(raw: &str, publisher_key: &str) -> Result<Self> {
        let decoded = KK::fd1(raw.trim(), publisher_key, true)?;
        serde_json::from_str(&decoded).map_err(|e| anyhow!("publisher.kxk JSON parse: {e}"))
    }

    pub fn kry_code(&self) -> KryCode {
        let pkxk = self.pkxkname.as_deref().unwrap_or("fernus");
        match self.fernus_code.as_deref() {
            Some(code) => {
                decrypt::parse_kry_code(code, pkxk.len()).unwrap_or(decrypt::DEFAULT_KRY_CODE)
            }
            None => decrypt::DEFAULT_KRY_CODE,
        }
    }
}

impl FromStr for PublisherConfig {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        Self::parse_with_key(raw, DEFAULT_PUBLISHER_KEY)
    }
}

/// Decrypt `sysd.dll` → XML configuration string.
pub fn decrypt_sysd(path: &Path, publisher_key: &str) -> Result<String> {
    let raw =
        std::fs::read_to_string(path).map_err(|e| anyhow!("reading {}: {}", path.display(), e))?;
    KK::fd1(raw.trim(), publisher_key, true)
}

/// Decrypt `sysb.dll`/`sysm.dll` → uncompressed FWS SWF bytes (header rewritten
/// so consumers parse directly without an extra zlib pass).
pub fn decrypt_swf_asset(path: &Path, code: &KryCode) -> Result<Vec<u8>> {
    let mut bytes =
        std::fs::read(path).map_err(|e| anyhow!("reading {}: {}", path.display(), e))?;
    KrySWFCrypto::decrypt(&mut bytes, code)?;

    if bytes.len() < 8 {
        bail!("decrypted SWF too short: {} bytes", bytes.len());
    }

    let declared_total = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;

    match &bytes[..3] {
        b"FWS" => Ok(bytes),
        b"CWS" | b"cws" => {
            use std::io::Read;
            let mut decoder = flate2::read::ZlibDecoder::new(&bytes[8..]);
            let mut body = Vec::with_capacity(declared_total.saturating_sub(8));
            decoder
                .read_to_end(&mut body)
                .map_err(|e| anyhow!("zlib inflate: {e}"))?;

            let mut fws = Vec::with_capacity(8 + body.len());
            fws.extend_from_slice(b"FWS");
            fws.push(bytes[3]);
            fws.extend_from_slice(&(declared_total as u32).to_le_bytes());
            fws.extend_from_slice(&body);
            Ok(fws)
        }
        b"ZWS" | b"zws" => bail!("ZWS (LZMA) SWF not yet supported for {}", path.display()),
        sig => bail!("not a SWF (sig={:?})", std::str::from_utf8(sig).ok()),
    }
}

/// Convenience: decrypt `sysb.dll` to an in-memory FWS SWF.
pub fn decrypt_pages(assets: &FernusAssets, code: &KryCode) -> Result<Vec<u8>> {
    let path = assets
        .sysb
        .as_ref()
        .ok_or_else(|| anyhow!("sysb.dll not present in {}", assets.root.display()))?;
    decrypt_swf_asset(path, code)
}
