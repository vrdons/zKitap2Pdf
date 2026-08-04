//! Conversion pipeline orchestration.
//!
//! Supports three Fernus formats transparently:
//!
//! **V1 (.dll):** Each DLL is one multi-frame SWF, decrypted via KrySWFCrypto
//!   or KK::fd1.
//!
//! **V2 (.frns):** Flash-LZMA-compressed JSON containing per-frame encrypted
//!   SWF data. Each frame is independently XOR-decoded, base64-decoded, then
//!   KrySWFCrypto-decrypted into a single-frame CWS SWF.
//!
//! **V3 (Enigma + Flutter):** EXE packed with Enigma Virtual Box containing
//!   Flutter/Dart assets (kernel_blob.bin + encrypted webp pages). Decrypts
//!   via envied XOR → AES-256-CBC and converts webp → PDF.
//!
//! KryCode is resolved from `sysd.frns` / `sysd.dll` / `publisher.kxk`,
//! falling back to the verified `{33, 20, 10}` default.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tempfile::TempDir;

use crate::cli::Files;
use crate::fernus::assets::PublisherConfig;
use crate::fernus::crypto::{DEFAULT_KRY_CODE, DEFAULT_PUBLISHER_KEY, KK, KryCode, KrySWFCrypto};
use crate::fernus::frns::{self, DecryptedFrame};
use crate::fernus::v3;
use crate::ruffle::exporter::Exporter;
use crate::ruffle::render::{SwfInput, render as render_pdf};
use crate::ruffle::swf;
use crate::utils::{self, process, watcher};

/// Process one EXE end-to-end. Auto-detects v1/v2 vs v3 format.
pub fn handle_exe(exporter: &Exporter, file: &Files, scale: f64) -> Result<()> {
    if utils::has_enigma(&file.input) {
        tracing::info!("v3 format detected (Enigma + Flutter)");
        return handle_v3(file, scale);
    }

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

/// Process a v3 Flutter + Enigma EXE.
fn handle_v3(file: &Files, _scale: f64) -> Result<()> {
    // Step 1: Unpack Enigma VFS
    tracing::info!("[1/6] Unpacking Enigma VFS...");
    let extracted = crate::enigma::unpack(&file.input).context("Enigma unpack failed")?;
    tracing::info!(file_count = extracted.len(), "VFS extracted");

    // Build lookup by path
    let lookup: HashMap<&str, &[u8]> = extracted
        .iter()
        .map(|f| (f.path.as_str(), f.data.as_slice()))
        .collect();

    // Debug: dump VFS file list
    tracing::debug!("VFS paths:");
    for path in lookup.keys() {
        let data = lookup[path];
        tracing::debug!(path, len = data.len(), first_bytes = ?&data[..data.len().min(16)]);
    }

    // Step 2: Extract keystr & IV from kernel_blob.bin
    tracing::info!("[2/6] Extracting keystr & IV from kernel_blob.bin...");
    let kernel = lookup
        .get("data/flutter_assets/kernel_blob.bin")
        .or_else(|| lookup.get("data\\flutter_assets\\kernel_blob.bin"))
        .ok_or_else(|| anyhow!("kernel_blob.bin not found in VFS"))?;

    tracing::debug!(kernel_len = kernel.len(), kernel_first = ?&kernel[..kernel.len().min(64)]);
    // Check if kernel starts with a known header
    if let Ok(s) = std::str::from_utf8(&kernel[..kernel.len().min(200)]) {
        tracing::debug!(kernel_preview = %s);
    }

    let keystr = v3::extract_keystr(kernel).context("extract keystr")?;
    let iv_str = v3::extract_iv(kernel).context("extract IV")?;
    tracing::info!(keystr = %keystr, iv = %iv_str, "credentials extracted");

    let iv = iv_str.as_bytes();
    let keystr_bytes = keystr.as_bytes();

    // Step 3: Decrypt publisher.json → fernusCode
    tracing::info!("[3/6] Decrypting publisher.json...");
    let pub_enc = lookup
        .get("publisher/publisher.json")
        .or_else(|| lookup.get("publisher\\publisher.json"))
        .ok_or_else(|| anyhow!("publisher.json not found in VFS"))?;
    let pub_enc_str = std::str::from_utf8(pub_enc).context("publisher.json not UTF-8")?;

    let pub_json_bytes = v3::decrypt_string(pub_enc_str.trim(), keystr_bytes, iv)
        .context("decrypt publisher.json")?;
    let pub_json_str = String::from_utf8(pub_json_bytes).context("publisher.json not UTF-8")?;

    let pub_cfg: serde_json::Value =
        serde_json::from_str(&pub_json_str).context("publisher.json parse")?;

    let fernus_code_enc = pub_cfg["fernusCode"]
        .as_str()
        .ok_or_else(|| anyhow!("fernusCode not found in publisher.json"))?;
    let fernus_code = String::from_utf8(
        v3::decrypt_string(fernus_code_enc, keystr_bytes, iv)
            .context("decrypt fernusCode")?,
    )
    .context("fernusCode not UTF-8")?;

    tracing::info!(fernus_code = %fernus_code, "fernusCode decrypted");


    // Step 4: Create book key
    tracing::info!("[4/6] Creating book key...");
    let book_key = v3::create_key(&fernus_code, &keystr);
    tracing::info!(book_key_len = book_key.len(), "book key ready");

    // Step 5: Decrypt book.json
    tracing::info!("[5/6] Decrypting book metadata & assets...");
    let book_enc = lookup
        .get("publisher/book/book.json")
        .or_else(|| lookup.get("publisher\\book\\book.json"))
        .ok_or_else(|| anyhow!("book.json not found in VFS"))?;
    let book_enc_str = std::str::from_utf8(book_enc).context("book.json not UTF-8")?;

    let book_json_bytes = v3::decrypt_string(book_enc_str.trim(), &book_key, iv)
        .context("decrypt book.json")?;
    let book_json_str = String::from_utf8(book_json_bytes).context("book.json not UTF-8")?;
    let book_data: serde_json::Value =
        serde_json::from_str(&book_json_str).context("book.json parse")?;

    let book_name = book_data["bookName"]
        .as_str()
        .unwrap_or("unknown");
    let total_page = book_data["totalPage"].as_u64().unwrap_or(0);
    tracing::info!(book_name, total_page, "book metadata");

    // Collect and decrypt webp files from the VFS
    let (pages, layers, _thumbs) = collect_v3_webp(&extracted, &book_key, iv)?;
    tracing::info!(pages = pages.len(), layers = layers.len(), "assets decrypted");

    // Step 6: webp → PDF
    tracing::info!("[6/6] Converting webp → PDF...");
    let pdf_path = file.output.clone();
    if let Some(parent) = pdf_path.parent() {
        fs::create_dir_all(parent)?;
    }
    webp_to_pdf(&pages, &layers, &pdf_path)?;
    tracing::info!(output = %pdf_path.display(), "PDF written");

    Ok(())
}

/// Collect, decrypt, and categorise webp files from extracted VFS.
fn collect_v3_webp(
    extracted: &[crate::enigma::ExtractedFile],
    book_key: &[u8],
    iv: &[u8],
) -> Result<(Vec<Vec<u8>>, HashMap<usize, Vec<u8>>, Vec<Vec<u8>>)> {
    let mut pages: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut layers: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut thumbs: Vec<Vec<u8>> = Vec::new();

    for f in extracted {
        let path = &f.path;
        let fn_lower = path.to_lowercase();

        if !fn_lower.ends_with(".webp") {
            continue;
        }

        // Extract filename from path
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let name_lower = name.to_lowercase();

        // Thumbnails: t-*.webp — unencrypted (RIFF header)
        if name_lower.starts_with("t-") && f.data.len() >= 4 && &f.data[..4] == b"RIFF" {
            thumbs.push(f.data.clone());
            continue;
        }

        // Try AES decrypt
        let dec = match v3::decrypt_bytes(&f.data, book_key, iv) {
            Ok(d) => d,
            Err(_) => {
                // Maybe it's already plain (RIFF header)
                if f.data.len() >= 4 && &f.data[..4] == b"RIFF" {
                    f.data.clone()
                } else {
                    tracing::warn!(path = %path, "decrypt failed, skipping");
                    continue;
                }
            }
        };

        // p-l-N.webp = layer for page N
        if name_lower.starts_with("p-l-") {
            if let Some(num) = extract_page_number(name) {
                layers.insert(num, dec);
            }
            continue;
        }

        // p-N.webp = page N
        if name_lower.starts_with("p-") {
            if let Some(num) = extract_page_number(name) {
                pages.push((num, dec));
            }
            continue;
        }

        // Unknown webp, skip
        tracing::debug!(path = %path, "unknown webp, skipped");
    }

    // Sort pages by number
    pages.sort_by_key(|(n, _)| *n);

    let pages: Vec<Vec<u8>> = pages.into_iter().map(|(_, d)| d).collect();
    Ok((pages, layers, thumbs))
}

fn extract_page_number(name: &str) -> Option<usize> {
    // p-1.webp or p-l-1.webp → extract "1"
    let stem = name.trim_end_matches(".webp").trim_end_matches(".WEBP");
    // Find the last '-' and parse what follows
    let pos = stem.rfind('-')?;
    stem[pos + 1..].parse::<usize>().ok()
}

/// Convert decrypted webp pages + optional layers to a PDF.
fn webp_to_pdf(
    pages: &[Vec<u8>],
    layers: &HashMap<usize, Vec<u8>>,
    pdf_path: &Path,
) -> Result<()> {
    use image::{DynamicImage, ImageFormat};

    if pages.is_empty() {
        // Create empty PDF
        let mut doc = oxidize_pdf::Document::new();
        return doc.save(pdf_path).context("save empty PDF");
    }

    let mut doc = oxidize_pdf::Document::new();
    doc.set_title(&file_stem(pdf_path));

    for (idx, page_data) in pages.iter().enumerate() {
        let mut img = image::load_from_memory_with_format(page_data, ImageFormat::WebP)
            .with_context(|| format!("decode webp page {idx}"))?
            .to_rgb8();

        // Merge layer if exists
        if let Some(layer_data) = layers.get(&idx) {
            let layer = image::load_from_memory_with_format(layer_data, ImageFormat::WebP)
                .with_context(|| format!("decode webp layer {idx}"))?
                .to_rgba8();

            // Paste RGBA layer onto RGB page
            for y in 0..layer.height().min(img.height()) {
                for x in 0..layer.width().min(img.width()) {
                    let lp = layer.get_pixel(x, y);
                    if lp[3] > 0 {
                        img.put_pixel(x, y, image::Rgb([lp[0], lp[1], lp[2]]));
                    }
                }
            }
        }

        // Encode as JPEG for PDF
        let mut jpg_buf = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut jpg_buf), ImageFormat::Jpeg)
            .with_context(|| format!("encode jpg page {idx}"))?;

        let pdf_image = oxidize_pdf::Image::from_jpeg_data(jpg_buf)
            .with_context(|| format!("pdf image page {idx}"))?;

        let mut page = oxidize_pdf::Page::new(
            pdf_image.width() as f64,
            pdf_image.height() as f64,
        );
        page.add_image("img", pdf_image);
        page.draw_image("img", 0.0, 0.0,
            page.width(), page.height(),
        )?;
        doc.add_page(page);
    }

    doc.save(pdf_path).context("save PDF")?;
    Ok(())
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string()
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
