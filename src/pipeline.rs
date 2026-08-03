//! Conversion pipeline orchestration.
//!
//! Supports two Fernus formats transparently:
//!
//! **V1 (.dll):** Each DLL is one multi-frame SWF, decrypted via KrySWFCrypto
//!   or KK::fd1.
//!
//! **V2 (.frns):** Flash-LZMA-compressed JSON containing per-frame encrypted
//!   SWF data. Each frame is independently XOR-decoded, base64-decoded, then
//!   KrySWFCrypto-decrypted into a single-frame CWS SWF.
//!
//! KryCode is resolved from `sysd.frns` / `sysd.dll` / `publisher.kxk`,
//! falling back to the verified `{33, 20, 10}` default.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use tempfile::TempDir;

use crate::cli::Files;
use crate::fernus::assets::PublisherConfig;
use crate::fernus::crypto::{DEFAULT_KRY_CODE, DEFAULT_PUBLISHER_KEY, KK, KryCode, KrySWFCrypto};
use crate::fernus::frns::{self, DecryptedFrame};
use crate::ruffle::exporter::Exporter;
use crate::ruffle::render::{SwfInput, render as render_pdf};
use crate::ruffle::swf;
use crate::utils::{self, process, watcher};

/// Process one EXE end-to-end.
pub fn handle_exe(exporter: &Exporter, file: &Files, scale: f64) -> Result<()> {
    let _child = process::execute_exe(&file.input)?;

    let temp_path = process::temp_path()?;
    tracing::debug!(temp = %temp_path.display(), "watching %TEMP%");

    let payloads = watcher::watch_and_collect(&temp_path)?;
    tracing::debug!(count = payloads.len(), keys = ?payloads.keys().collect::<Vec<_>>(), "collected");

    if payloads.is_empty() {
        return Err(anyhow!("No payload files found in Temp"));
    }

    // Resolve KryCode from whatever config source is available
    let kry_code = resolve_kry_code(&payloads);
    tracing::debug!(code = ?kry_code, "KryCode resolved");

    let swf_tmp = TempDir::new().context("create swf temp dir")?;
    let mut swf_inputs: Vec<SwfInput> = Vec::new();

    for (name, data) in &payloads {
        // Skip config files — they're only for KryCode resolution
        // Sysm is mask
        if name == "p.dll" || name.starts_with("sysd") || name.starts_with("sysm") || name == "publisher.kxk" {
            continue;
        }

        tracing::info!(name = %name, bytes = data.len(), "decrypting");

        if name.ends_with(".frns") {
            // --- V2: per-frame FRNS bundle ---
            let frames = match frns::load_and_decrypt_book(data, &kry_code) {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!(name = %name, error = %e, "frns decrypt failed");
                    continue;
                }
            };
            tracing::info!(name = %name, frame_count = frames.len(), "frns decrypted");
            for frame in &frames {
                if let Some(input) = write_frame_swf(&swf_tmp, name, frame) {
                    swf_inputs.push(input);
                }
            }
        } else {
            // --- V1: legacy DLL → one multi-frame SWF ---
            let fws = match try_decrypt_kry(data, &kry_code)
                .or_else(|| try_decrypt_fd1(data, DEFAULT_PUBLISHER_KEY))
            {
                Some(f) => f,
                None => {
                    tracing::debug!(name = %name, len = data.len(), head = ?hex::encode(&data[..data.len().min(16)]), "decrypt failed");
                    continue;
                }
            };
            let swf_path = swf_tmp.path().join(name);
            match write_patched_swf(&fws, &swf_path, 0.0, 0.0) {
                Ok(input) => swf_inputs.push(input),
                Err(e) => tracing::warn!(name = %name, error = %e, "patch/write failed"),
            }
        }
    }

    if swf_inputs.is_empty() {
        return Err(anyhow!("No decryptable SWF payloads found"));
    }

    swf_inputs.sort_by_key(|input| !input.name.contains("sysb"));
    render_pdf(exporter, &swf_inputs, file, scale)
}

// ---------------------------------------------------------------------------
// KryCode resolution (unified — probes all known config sources)
// ---------------------------------------------------------------------------

fn resolve_kry_code(payloads: &HashMap<String, Vec<u8>>) -> KryCode {
    let key = DEFAULT_PUBLISHER_KEY;

    // Ordered probes: sysd.frns → sysd.dll → publisher.kxk → default
    for config_name in &["sysd.frns", "sysd.dll", "publisher.kxk"] {
        let Some(data) = payloads.get(*config_name) else { continue };

        let text = String::from_utf8_lossy(data);
        let Ok(decrypted) = KK::fd1(text.trim(), key, true) else {
            tracing::debug!(source = config_name, "fd1 decrypt failed");
            continue;
        };

        // Try JSON (publisher.kxk / newer sysd)
        if let Ok(cfg) = serde_json::from_str::<PublisherConfig>(&decrypted) {
            let code = cfg.kry_code();
            tracing::debug!(code = ?code, source = config_name, "KryCode resolved");
            return code;
        }

        // Try XML (older sysd.frns / sysd.dll)
        if let Some(code) = extract_kry_code_from_xml(&decrypted) {
            tracing::debug!(code = ?code, source = config_name, "KryCode from XML");
            return code;
        }

        tracing::debug!(source = config_name, "fd1 ok but not valid JSON/XML");
    }

    tracing::debug!(code = ?DEFAULT_KRY_CODE, "using default KryCode");
    DEFAULT_KRY_CODE
}

fn extract_kry_code_from_xml(xml: &str) -> Option<KryCode> {
    let fernus_code = utils::xml_tag(xml, "fernusCode")?;
    let pkxkname = utils::xml_tag(xml, "pkxkname").unwrap_or("fernus");
    crate::fernus::crypto::parse_kry_code(fernus_code, pkxkname.len()).ok()
}


// ---------------------------------------------------------------------------
// SWF writing helpers
// ---------------------------------------------------------------------------

fn write_frame_swf(swf_tmp: &TempDir, bundle_name: &str, frame: &DecryptedFrame) -> Option<SwfInput> {
    let fws = match swf::to_fws(&frame.swf_bytes) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(name = %bundle_name, frame = frame.frame, error = %e, "to_fws failed, using raw");
            frame.swf_bytes.clone()
        }
    };

    let swf_name = format!("{bundle_name}_frame{:04}.swf", frame.frame);
    let dest = swf_tmp.path().join(&swf_name);

    let result = write_patched_swf(&fws, &dest, frame.width, frame.height);

    match result {
        Ok(input) => {
            tracing::debug!(name = %swf_name, "frame written");
            Some(input)
        }
        Err(e) => {
            tracing::warn!(name = %bundle_name, frame = frame.frame, error = %e, "patch/write failed");
            None
        }
    }
}

fn write_patched_swf(fws_bytes: &[u8], dest: &PathBuf, width: f64, height: f64) -> Result<SwfInput> {
    // If dims provided by caller (e.g. FRNS metadata), use fast path.
    // Otherwise scan DefineShape bounds.
    let (w, h) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        let (_, view) = swf::load(fws_bytes)?;
        (view.width, view.height)
    };
    let swf_buf = swf::decompress_swf_quick(fws_bytes)?;
    let patched = swf::patch(swf_buf, w, h)?;
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("unknown.swf").to_string();
    fs::write(dest, &patched)?;
    Ok(SwfInput { name, path: dest.clone(), width: w, height: h })
}

fn try_decrypt_kry(data: &[u8], code: &KryCode) -> Option<Vec<u8>> {
    if data.is_empty() { return None; }
    let mut bytes = data.to_vec();
    KrySWFCrypto::decrypt(&mut bytes, code).ok()?;
    swf::to_fws(&bytes).ok()
}

fn try_decrypt_fd1(data: &[u8], key: &str) -> Option<Vec<u8>> {
    if data.is_empty() { return None; }
    let text = String::from_utf8_lossy(data);
    let decrypted = KK::fd1(text.trim(), key, true).ok()?;
    swf::to_fws(decrypted.as_bytes()).ok()
}
