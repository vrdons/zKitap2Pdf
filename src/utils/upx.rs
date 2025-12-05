use core::panic;
use std::{fs, path::Path};

use crate::paths::TEMP_PATH;

pub async fn decompress(path: &str) -> anyhow::Result<String> {
    use tokio::process::Command;
    let file_name = Path::new(path)
        .file_name() // sadece dosya adını alır
        .and_then(|n| n.to_str()) // &str'e çevirir
        .ok_or_else(|| anyhow::anyhow!("Dosya adı alınamadı"))?;
    let output_path = Path::new(TEMP_PATH).join(file_name).display().to_string();
    let output = Command::new(crate::paths::UPX_PATH)
        .arg("-d")
        .arg(path)
        .arg("-o")
        .arg(&output_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("{}", stderr);
        if stderr.contains("NotPackedException") || stderr.contains("not packed by UPX") {
            eprintln!("upx: '{}' UPX ile paketli değil — atlanıyor", &path);
            fs::copy(&path, &output_path)
                .unwrap_or_else(|e| panic!("dosya kopyalanırken hata oluştu {}", e));
            return Ok(output_path);
        }
        return Err(anyhow::anyhow!(
            "UPX decompress failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output_path)
}
