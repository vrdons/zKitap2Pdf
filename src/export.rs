use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use image::{DynamicImage, ImageFormat};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use oxidize_pdf::{Document, Image, Page};
use tempfile::NamedTempFile;
use crate::decrypt::DEFAULT_PUBLISHER_KEY;

use crate::cli::Files;
use crate::decrypt::{KK, KryCode, KrySWFCrypto, DEFAULT_KRY_CODE};
use crate::exporter::Exporter;
use crate::fernus_assets::{self, FernusAssets, PublisherConfig};
use crate::pe_scanner;
use crate::utils;

#[derive(Debug, Clone)]
pub struct HandleArgs {
    pub file: Files,
    pub scale: f64,
    pub debug: bool,
}

pub fn handle_exe(exporter: &Exporter, args: &HandleArgs) -> Result<()> {
    let input_path = &args.file.input;
    let scale = args.scale;
    let debug = args.debug;

    let mut child = crate::executable::execute_exe(input_path)?;

    let temp_path = crate::executable::get_temp_path()?;
    if debug {
        eprintln!("[debug] Watching Temp: {:?}", temp_path);
    }
    let dlls = watch_temp_and_collect(&temp_path, debug)?;

    let _ = child.kill();

    if debug {
        eprintln!(
            "[debug] Collected {} DLL(s): {:?}",
            dlls.len(),
            dlls.keys().collect::<Vec<_>>()
        );
    }

    if dlls.is_empty() {
        println!("No DLLs found in Temp, trying external assets...");
        return render_external_assets(exporter, args);
    }

    let publisher_key = dlls
        .get("p.dll")
        .and_then(|data| extract_publisher_key(data))
        .unwrap_or_else(|| DEFAULT_PUBLISHER_KEY.to_string());
    println!("Publisher key: \"{publisher_key}\"");

    let kry_code = dlls
        .get("sysd.dll")
        .and_then(|data| {
            let text = String::from_utf8_lossy(data);
            KK::fd1(text.trim(), &publisher_key, true).ok()
        })
        .and_then(|decrypted| serde_json::from_str::<PublisherConfig>(&decrypted).ok())
        .map(|cfg| cfg.kry_code())
        .unwrap_or(DEFAULT_KRY_CODE);
    if debug {
        eprintln!("[debug] KryCode: {:?}", kry_code);
    }

    let mut swf_data: Vec<(String, Vec<u8>)> = Vec::new();

    for (name, data) in &dlls {
        if name == "p.dll" {
            continue;
        }

        if debug {
            eprintln!("[debug] Decrypting: {} ({} bytes)", name, data.len());
        }

        if let Some(fws) = try_decrypt_kry(data, &kry_code) {
            if debug {
                eprintln!("[debug]   -> KrySWFCrypto OK: {} bytes FWS", fws.len());
            }
            swf_data.push((name.clone(), fws));
            continue;
        }

        if let Some(fws) = try_decrypt_fd1(data, &publisher_key) {
            if debug {
                eprintln!("[debug]   -> KK::fd1 OK: {} bytes FWS", fws.len());
            }
            swf_data.push((name.clone(), fws));
            continue;
        }

        if debug {
            eprintln!("[debug]   -> FAILED, skipping");
        }
    }

    if swf_data.is_empty() {
        return Err(anyhow!(
            "No decryptable SWF pages found in {} DLL(s) from Temp",
            dlls.len()
        ));
    }

    swf_data.sort_by(|a, b| {
        let a_sysb = a.0.contains("sysb");
        let b_sysb = b.0.contains("sysb");
        b_sysb.cmp(&a_sysb)
    });

    render_swf_data(exporter, &swf_data, &args.file, scale)
}

fn watch_temp_and_collect(temp_path: &std::path::Path, debug: bool) -> Result<HashMap<String, Vec<u8>>> {
    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, Config::default()).map_err(|e| anyhow!("notify: {e}"))?;
    watcher
        .watch(temp_path, RecursiveMode::NonRecursive)
        .map_err(|e| anyhow!("watch: {e}"))?;

    let mut dlls: HashMap<String, Vec<u8>> = HashMap::new();
    let mut last_activity = Instant::now();
    let idle_timeout = Duration::from_secs(30);
    let max_wait = Duration::from_secs(120);
    let start = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };

                    if debug {
                    eprintln!("[debug] path={}, kind={:?}", name.to_string(), event.kind);
                    }

                    if !name.ends_with(".dll") {
                        continue;
                    }
                    let name = name.to_string();
                    if dlls.contains_key(&name) {
                        continue;
                    }
                    match fs::read(path) {
                        Ok(data) => {
                            if debug {
                                eprintln!(
                                    "[debug] ({:.1} KB)",
                                    data.len() as f64 / 1024.0,
                                );
                            }
                            dlls.insert(name, data);
                            last_activity = Instant::now();
                        }
                        Err(e) => {
                            if debug {
                                eprintln!("[debug] Temp: read {} → {e}", name);
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                if debug {
                    eprintln!("[debug] notify error: {e}");
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !dlls.is_empty() && last_activity.elapsed() >= idle_timeout {
                    if debug {
                        eprintln!("[debug] Idle {:.0}s, stopping watch", last_activity.elapsed().as_secs());
                    }
                    break;
                }
                if start.elapsed() >= max_wait {
                    if debug {
                        eprintln!("[debug] Max wait reached, stopping watch");
                    }
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(dlls)
}

fn try_decrypt_kry(data: &[u8], code: &KryCode) -> Option<Vec<u8>> {
    let mut bytes = data.to_vec();
    KrySWFCrypto::decrypt(&mut bytes, code).ok()?;
    convert_to_fws(&bytes)
}

fn try_decrypt_fd1(data: &[u8], key: &str) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(data);
    let decrypted = KK::fd1(text.trim(), key, true).ok()?;
    convert_to_fws(decrypted.as_bytes())
}

fn convert_to_fws(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }
    match &data[..3] {
        b"FWS" => Some(data.to_vec()),
        b"CWS" | b"cws" => {
            let declared = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
            let mut decoder = flate2::read::ZlibDecoder::new(&data[8..]);
            let mut body = Vec::with_capacity(declared.saturating_sub(8));
            decoder.read_to_end(&mut body).ok()?;
            let mut fws = Vec::with_capacity(8 + body.len());
            fws.extend_from_slice(b"FWS");
            fws.push(data[3]);
            fws.extend_from_slice(&(declared as u32).to_le_bytes());
            fws.extend_from_slice(&body);
            Some(fws)
        }
        _ => None,
    }
}

fn extract_publisher_key(raw_data: &[u8]) -> Option<String> {
    if let Ok(Some(found)) = pe_scanner::find_abc_string_containing(raw_data, "pub1isher") {
        return Some(found);
    }
    if let Ok(Some(found)) = pe_scanner::find_abc_string_containing(raw_data, "isher") {
        if found.to_lowercase().contains("pub") {
            return Some(found);
        }
    }
    if let Ok(strings) = pe_scanner::extract_abc_strings(raw_data) {
        for s in &strings {
            let lower = s.to_lowercase();
            if lower.contains("pub") && lower.contains("ish") && s.len() >= 10 && s.len() <= 20 {
                return Some(s.clone());
            }
        }
    }
    println!("Use default publisher key");
    None
}

fn render_swf_data(
    exporter: &Exporter,
    swf_data: &[(String, Vec<u8>)],
    file_info: &Files,
    scale: f64,
) -> Result<()> {
    let mut doc = Document::new();
    doc.set_title(&file_info.filename);
    doc.set_author("Vrdons <vrdons@proton.me>");

    let mut total_pages = 0u32;
    let mut jpeg_file = NamedTempFile::new()?;

    for (idx, (name, fws_bytes)) in swf_data.iter().enumerate() {
        let (width, height) = utils::find_real_size_from_bytes(fws_bytes)?;
        let swf_buf = swf::decompress_swf(&mut Cursor::new(fws_bytes))
            .map_err(|e| anyhow!("decompress SWF: {e}"))?;
        let frame_count = utils::frame_count_from_fws(fws_bytes)? as u32;
        let patched = utils::patch_swf(swf_buf, width, height)?;

        let patched_file = NamedTempFile::new()?;
        fs::write(patched_file.path(), &patched)?;

        let scaled_width = (width * scale).round();
        let scaled_height = (height * scale).round();
        println!(
            "  [{}/{}] {}  {:.0}x{:.0} → {:.0}x{:.0}  {} frames",
            idx + 1,
            swf_data.len(),
            name,
            width,
            height,
            scaled_width,
            scaled_height,
            frame_count
        );

        let mut rendered = 0u32;

        exporter.capture_frames(patched_file.path(), |_frame_idx, image| {
            // Encode to JPEG in memory → single reused temp file
            let rgb = DynamicImage::ImageRgba8(image).to_rgb8();
            let mut jpeg_buf = Cursor::new(Vec::with_capacity(rgb.len() / 3));
            if rgb.write_to(&mut jpeg_buf, ImageFormat::Jpeg).is_err() {
                return;
            }
            let jpeg_bytes = jpeg_buf.into_inner();

            // Reuse single temp file (truncate + rewrite)
            let _ = jpeg_file.as_file().set_len(0);
            let _ = jpeg_file.as_file().seek(SeekFrom::Start(0));
            if jpeg_file.write_all(&jpeg_bytes).is_err() {
                return;
            }
            let _ = jpeg_file.as_file().flush();

            let pdf_image = match Image::from_jpeg_file(jpeg_file.path()) {
                Ok(img) => img,
                Err(_) => return,
            };

            let mut page = Page::new(scaled_width, scaled_height);
            page.add_image("img", pdf_image);
            let _ = page.draw_image("img", 0.0, 0.0, scaled_width, scaled_height);
            doc.add_page(page);
            rendered += 1;

            if rendered.is_multiple_of(200) {
                println!("    ... {rendered}/{frame_count}");
            }
        })?;

        total_pages += rendered;
        println!("    ✓ {rendered} frames from {name}");
    }

    drop(jpeg_file);

    println!(
        "Saving PDF ({total_pages} pages) → {:?}",
        file_info.output
    );
    if let Some(parent) = file_info.output.parent() {
        fs::create_dir_all(parent)?;
    }
    doc.save(&file_info.output)?;
    println!("Done! {total_pages} total pages.");
    Ok(())
}

// ── Fallback: external assets (wine-free path) ─────────────────────────────

fn render_external_assets(exporter: &Exporter, args: &HandleArgs) -> Result<()> {
    let input_path = &args.file.input;
    let scale = args.scale;

    let raw_bytes = fs::read(input_path)?;
    let publisher_key = extract_publisher_key(&raw_bytes)
        .unwrap_or_else(|| DEFAULT_PUBLISHER_KEY.to_string());
    println!("Publisher key (from EXE): \"{publisher_key}\"");

    let assets = FernusAssets::find(input_path);
    if !assets.has_pages() {
        return Err(anyhow!(
            "No DLLs in Temp and no sysb.dll found next to EXE.\n\
             Make sure Wine is configured and the EXE runs correctly."
        ));
    }

    let code = assets
        .publisher
        .as_ref()
        .and_then(|p| PublisherConfig::from_file(p, &publisher_key).ok())
        .map(|cfg| cfg.kry_code())
        .unwrap_or(DEFAULT_KRY_CODE);

    let page_fws = fernus_assets::decrypt_pages(&assets, &code)?;
    let swf_data = vec![("sysb.dll".to_string(), page_fws)];
    render_swf_data(exporter, &swf_data, &args.file, scale)
}
