//! Conversion pipeline orchestration (was `handle_exe` in `export.rs`).
//!
//! Per-EXE flow:
//!   1. Launch the projector (`process::execute_exe`) and watch its `%TEMP%`
//!      drop dir (`watcher::watch_and_collect`).
//!   2. Resolve the KryCode from the publisher payload, falling back to the
//!      verified default for Isler-4065.
//!   3. Decrypt each runtime DLL (`sysb`/`sysm`/`sysd`) trying KrySWFCrypto
//!      first, then KK::fd1; normalise to FWS.
//!   4. Hand the decrypted SWF batches to `render::render`, which drives the
//!      Ruffle frame capture and assembles the PDF.
//!
//! `sysb` is sorted first because it carries the actual page content; masks
//! and config must follow so the page order in the PDF stays stable.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::cli::Files;
use crate::fernus::assets::PublisherConfig;
use crate::fernus::crypto::{DEFAULT_KRY_CODE, DEFAULT_PUBLISHER_KEY, KK, KryCode, KrySWFCrypto};
use crate::ruffle::exporter::Exporter;
use crate::ruffle::render::render as render_pdf;
use crate::ruffle::swf;
use crate::utils::process;
use crate::utils::watcher;

/// Process one EXE end-to-end: decrypt runtime assets and render the PDF.
pub fn handle_exe(exporter: &Exporter, file: &Files, scale: f64) -> Result<()> {
    // Launch the projector. Its runtime drops sys*.dll + p.dll into %TEMP%.
    // NOTE: the wine-launched process spawns grandchildren we cannot reliably
    // track, so we deliberately leak the `Child` handle and let the user kill
    // the projector themselves. See analyzes/v1.md §4.1 for context.
    let _child = process::execute_exe(&file.input)?;

    let temp_path = process::temp_path()?;
    tracing::debug!(temp = %temp_path.display(), "watching %TEMP%");

    let dlls = watcher::watch_and_collect(&temp_path)?;
    tracing::debug!(count = dlls.len(), keys = ?dlls.keys().collect::<Vec<_>>(), "collected");

    if dlls.is_empty() {
        return Err(anyhow!("No DLLs found in Temp"));
    }
    tracing::info!(key = DEFAULT_PUBLISHER_KEY, "publisher");

    let kry_code = resolve_kry_code(&dlls, DEFAULT_PUBLISHER_KEY);
    tracing::debug!(code = ?kry_code, "KryCode resolved");

    tracing::info!("starting decrypt");
    let mut swf_data: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, data) in &dlls {
        if name == "p.dll" {
            //TODO: Make it actually decrypt
            continue;
        }
        tracing::debug!(name = %name, bytes = data.len(), "decrypting");

        if let Some(fws) = try_decrypt_kry(data, &kry_code) {
            tracing::debug!(name = %name, kind = "fws", size = fws.len(), "decrypt ok");
            swf_data.push((name.clone(), fws));
            continue;
        }
        if let Some(fws) = try_decrypt_fd1(data, DEFAULT_PUBLISHER_KEY) {
            tracing::debug!(name = %name, kind = "fd1", size = fws.len(), "decrypt ok");
            swf_data.push((name.clone(), fws));
            continue;
        }
        tracing::debug!(
            name = %name,
            len = data.len(),
            head = ?hex::encode(&data[..data.len().min(16)]),
            "decrypt failed"
        );
    }

    if swf_data.is_empty() {
        return Err(anyhow!("No decryptable SWF payloads found"));
    }

    // sysb holds the page content; sort it first so PDF page order is stable.
    swf_data.sort_by_key(|(name, _)| !name.contains("sysb"));

    render_pdf(exporter, &swf_data, file, scale)
}

/// Resolve the KryCode for this book from its publisher payload, falling back
/// to the verified Isler-4065 default when the payload is absent or malformed.
fn resolve_kry_code(dlls: &HashMap<String, Vec<u8>>, key: &str) -> KryCode {
    dlls.get("sysd.dll")
        .and_then(|data| {
            let text = String::from_utf8_lossy(data);
            KK::fd1(text.trim(), key, true).ok()
        })
        .and_then(|decrypted| serde_json::from_str::<PublisherConfig>(&decrypted).ok())
        .map(|cfg| cfg.kry_code())
        .unwrap_or(DEFAULT_KRY_CODE)
}

/// Attempt KrySWFCrypto byte-scramble decryption, then normalise to FWS.
fn try_decrypt_kry(data: &[u8], code: &KryCode) -> Option<Vec<u8>> {
    if data.is_empty() {
        tracing::debug!("kry: empty input, skipping");
        return None;
    }
    let mut bytes = data.to_vec();
    match KrySWFCrypto::decrypt(&mut bytes, code) {
        Ok(()) => {
            tracing::debug!(len = bytes.len(), head = ?hex::encode(&bytes[..bytes.len().min(16)]), "kry: decrypt ok");
        }
        Err(e) => {
            tracing::debug!(error = %e, "kry: decrypt failed");
            return None;
        }
    }
    match swf::to_fws(&bytes) {
        Ok(fws) => {
            tracing::debug!(len = fws.len(), "kry: to_fws ok");
            Some(fws)
        }
        Err(e) => {
            tracing::debug!(error = %e, "kry: to_fws failed");
            None
        }
    }
}

/// Attempt KK::fd1 string decryption, then normalise to FWS.
fn try_decrypt_fd1(data: &[u8], key: &str) -> Option<Vec<u8>> {
    if data.is_empty() {
        tracing::debug!("fd1: empty input, skipping");
        return None;
    }
    let text = String::from_utf8_lossy(data);
    let decrypted = match KK::fd1(text.trim(), key, true) {
        Ok(d) => {
            tracing::debug!(
                len = d.len(),
                head = &d[..d.len().min(64)],
                "fd1: decrypt ok"
            );
            d
        }
        Err(e) => {
            tracing::debug!(error = %e, "fd1: decrypt failed");
            return None;
        }
    };
    match swf::to_fws(decrypted.as_bytes()) {
        Ok(fws) => {
            tracing::debug!(len = fws.len(), "fd1: to_fws ok");
            Some(fws)
        }
        Err(e) => {
            tracing::debug!(error = %e, "fd1: to_fws failed");
            None
        }
    }
}
