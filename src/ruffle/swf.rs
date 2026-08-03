//! SWF byte-level helpers: format normalisation, single-pass analysis, header patching.
use std::io::{Cursor, Read};

use anyhow::{Context, Result, anyhow, bail};
use swf::{Header, Rectangle, SwfBuf, Tag, Twips, parse_swf, write::write_swf_raw_tags};

/// Minimum bytes needed to inspect a SWF signature + declared length field.
const SWF_HEADER_MIN: usize = 8;

/// Normalise a SWF byte stream to uncompressed FWS.
///
/// - `FWS` is returned as-is.
/// - `CWS`/`cws` is inflated with zlib and the header signature is rewritten
///   to `FWS`.
/// - `ZWS`/`zws` (LZMA) is rejected — the projector never produces it.
///
/// This used to live, byte-for-byte identical, in both `export.rs
/// ::convert_to_fws` and `fernus/assets.rs::decrypt_pages`.
pub fn to_fws(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < SWF_HEADER_MIN {
        bail!("SWF too short: {} bytes", data.len());
    }

    let declared_total = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let version = data[3];

    match &data[..3] {
        b"FWS" => Ok(data.to_vec()),
        b"CWS" | b"cws" => {
            let mut decoder = flate2::read::ZlibDecoder::new(&data[SWF_HEADER_MIN..]);
            let mut body = Vec::with_capacity(declared_total.saturating_sub(SWF_HEADER_MIN));
            decoder
                .read_to_end(&mut body)
                .with_context(|| "zlib inflate failed")?;

            let mut fws = Vec::with_capacity(SWF_HEADER_MIN + body.len());
            fws.extend_from_slice(b"FWS");
            fws.push(version);
            fws.extend_from_slice(&(declared_total as u32).to_le_bytes());
            fws.extend_from_slice(&body);
            Ok(fws)
        }
        b"ZWS" | b"zws" => bail!("ZWS (LZMA) SWF is not supported"),
        sig => {
            let sig_display = std::str::from_utf8(sig).unwrap_or("<non-utf8>");
            bail!("not a SWF signature: {sig_display:?}")
        }
    }
}

/// Decompress a SWF **once** and extract the metadata the renderer needs.
///
/// Returns the parsed [`SwfBuf`] (for later [`patch`]ing) alongside the real
/// stage size and frame count. The real size is computed by scanning
/// [`Tag::DefineShape`] bounds and taking the max — projector SWFs ship with a
/// zero/tiny stage rectangle, the actual dimensions live in shape bounds.
#[derive(Debug, Clone, Copy)]
pub struct SwfView {
    /// Real pixel width (max of DefineShape x_max, clamped to header stage size).
    pub width: f64,
    /// Real pixel height (max of DefineShape y_max, clamped to header stage size).
    pub height: f64,
    /// Declared frame count from the SWF header.
    pub frame_count: u16,
}

/// Decompress the SWF and extract its metadata plus the reusable buffer.
///
/// The returned [`SwfBuf`] can be passed straight to [`patch`].
pub fn load(data: &[u8]) -> Result<(SwfBuf, SwfView)> {
    let buf = swf::decompress_swf(&mut Cursor::new(data))
        .map_err(|e| anyhow!("failed to decompress SWF: {e}"))
        .context("swf load")?;
    let parsed = parse_swf(&buf).context("swf parse")?;

    let mut width = parsed.header.stage_size().x_max.to_pixels();
    let mut height = parsed.header.stage_size().y_max.to_pixels();
    for tag in &parsed.tags {
        if let Tag::DefineShape(shape) = tag {
            let b = shape.shape_bounds;
            width = width.max(b.x_max.to_pixels());
            height = height.max(b.y_max.to_pixels());
        }
    }

    let view = SwfView {
        width,
        height,
        frame_count: parsed.header.num_frames(),
    };
    Ok((buf, view))
}

/// Rewrite a SWF header's stage rectangle to a concrete pixel size.
///
/// `width`/`height` must be positive finite values. The compressed tag stream
/// is left untouched; only the header rectangle is rewritten.
pub fn patch(file: SwfBuf, width: f64, height: f64) -> Result<Vec<u8>> {
    if !width.is_finite() || width <= 0.0 {
        bail!("invalid patch width: {width} (must be positive finite)");
    }
    if !height.is_finite() || height <= 0.0 {
        bail!("invalid patch height: {height} (must be positive finite)");
    }

    let header = Header {
        version: file.header.version(),
        compression: file.header.compression(),
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(width),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(height),
        },
        frame_rate: file.header.frame_rate(),
        num_frames: file.header.num_frames(),
    };

    let mut out = Cursor::new(Vec::<u8>::new());
    write_swf_raw_tags(&header, &file.data, &mut out).context("swf re-encode")?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_input() {
        let err = to_fws(b"FW").unwrap_err();
        assert!(format!("{err}").contains("too short"));
    }

    #[test]
    fn rejects_zws() {
        let mut data = vec![0u8; 32];
        data[0..3].copy_from_slice(b"ZWS");
        let err = to_fws(&data).unwrap_err();
        assert!(format!("{err}").contains("ZWS"));
    }
}
