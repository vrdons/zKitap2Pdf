use flate2::read::ZlibDecoder;
use std::fs::File;
use std::io::Read;

pub fn decrypt_cws(file: &mut File) -> anyhow::Result<Vec<u8>> {
    let mut header = [0u8; 3];
    file.read_exact(&mut header)?;
    if &header != b"CWS" && &header != b"FWS" {
        panic!("Dosya CWS değil!");
    }
    let mut version = [0u8; 1];
    file.read_exact(&mut version)?;
    let mut file_length = [0u8; 4];
    file.read_exact(&mut file_length)?;
    let mut compressed = Vec::new();
    file.read_to_end(&mut compressed)?;
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    let mut swf_data = Vec::new();
    swf_data.extend_from_slice(b"FWS");
    swf_data.extend_from_slice(&version);
    swf_data.extend_from_slice(&file_length);
    swf_data.extend_from_slice(&decompressed);
    Ok(swf_data)
}
