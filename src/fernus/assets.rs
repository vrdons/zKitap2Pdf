//! Fernus asset helpers used when processing exported runtime bundles.

use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::crypto::{DEFAULT_PUBLISHER_KEY, KK, KryCode, KrySWFCrypto, parse_kry_code};
use crate::ruffle::swf;

/// Parsed publisher configuration from a Fernus publisher payload.
///
/// Fields beyond `pkxkname`/`fernus_code` are captured into `other` for
/// debugging; the publisher file-loaders are reserved for the non-Wine direct
/// consumer (see [`AssetBundle`]).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PublisherConfig {
    #[serde(default)]
    pub pkxkname: Option<String>,
    #[serde(default)]
    pub fernus_code: Option<String>,
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, serde_json::Value>,
}

#[allow(dead_code)]
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
        serde_json::from_str(&decoded).map_err(|e| anyhow!("publisher JSON parse: {e}"))
    }

    pub fn kry_code(&self) -> KryCode {
        let pkxk = self.pkxkname.as_deref().unwrap_or("fernus");
        match self.fernus_code.as_deref() {
            Some(code) => {
                parse_kry_code(code, pkxk.len()).unwrap_or(super::crypto::DEFAULT_KRY_CODE)
            }
            None => super::crypto::DEFAULT_KRY_CODE,
        }
    }
}

/// A small bundle view over the runtime payload files discovered in a book
/// folder or temp directory.
///
/// Reserved for the "direct decrypt from shipped assets" path
/// (`assets/*.dll` next to the projector, see `analyzes/v1.md` §1.1). The
/// active temp-folder pipeline in `pipeline.rs` does not use it yet, but it is
/// kept available as the natural API for a non-Wine consumer.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct AssetBundle {
    pub root: PathBuf,
    pub sysd: Option<PathBuf>,
    pub sysb: Option<PathBuf>,
    pub sysm: Option<PathBuf>,
    pub publisher: Option<PathBuf>,
}

#[allow(dead_code)]
impl AssetBundle {
    /// Build a bundle by probing the well-known runtime filenames in `dir`.
    pub fn from_dir(dir: &Path) -> Self {
        let lookup = |name: &str| {
            let p = dir.join(name);
            p.exists().then_some(p)
        };
        Self {
            root: dir.to_path_buf(),
            sysd: lookup("sysd.dll"),
            sysb: lookup("sysb.dll"),
            sysm: lookup("sysm.dll"),
            publisher: lookup("p.dll"),
        }
    }

    /// Whether `sysb.dll` (the page content) is present in this bundle.
    pub fn has_pages(&self) -> bool {
        self.sysb.is_some()
    }
}

/// Decrypt a SWF payload from the runtime assets into in-memory FWS bytes.
///
/// Reserved for the direct path (see [`AssetBundle`]); the active pipeline
/// decrypts the watcher-collected bytes in `pipeline.rs`.
#[allow(dead_code)]
pub fn decrypt_pages(assets: &AssetBundle, code: &KryCode) -> Result<Vec<u8>> {
    let path = assets
        .sysb
        .as_ref()
        .ok_or_else(|| anyhow!("sysb.dll not present in {}", assets.root.display()))?;

    let mut bytes =
        std::fs::read(path).map_err(|e| anyhow!("reading {}: {}", path.display(), e))?;
    KrySWFCrypto::decrypt(&mut bytes, code);

    Ok(swf::to_fws(&bytes)?)
}
