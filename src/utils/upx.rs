pub async fn decompress(path: &str) -> anyhow::Result<()> {
    use tokio::process::Command;

    let output = Command::new(crate::paths::UPX_PATH)
        .arg("-d")
        .arg(path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        if stderr.contains("NotPackedException") || stderr.contains("not packed by UPX") {
            eprintln!("upx: '{}' UPX ile paketli değil — atlanıyor", &path);
            return Ok(());
        }
       return Err(anyhow::anyhow!(
            "UPX decompress failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}
