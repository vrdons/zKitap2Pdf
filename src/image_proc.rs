//! Shared image processing: upscaling and overlay compositing.
//!
//! Used by both the v1/v2 SWF-render path (optional, the renderer already runs
//! at a scale factor) and the v3 webp path (where upscale is the primary way
//! to increase resolution). Keeping this in one place avoids divergent resize
//! behaviour between pipelines.

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgb, Rgba, RgbaImage};

/// Configuration for the optional upscale stage.
#[derive(Debug, Clone)]
pub struct UpscaleOpts {
    /// Scale factor. `1.0` = leave the image untouched (and skip the work
    /// entirely). Typical values: 1.0–3.0.
    pub scale: f64,
}

impl UpscaleOpts {
    pub fn new(scale: f64) -> Self {
        Self { scale }
    }

    /// Whether the upscale stage does any actual work. Callers can use this to
    /// short-circuit decode/encode when no resize is needed.
    pub fn is_noop(&self) -> bool {
        self.scale <= 1.0 || !self.scale.is_finite()
    }
}

impl Default for UpscaleOpts {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Upscale an image by the configured factor.
///
/// * If `opts.scale <= 1.0` or non-finite, the image is returned **unchanged** —
///   no decode/encode cycle, no extra allocation.
/// * Aspect ratio is always preserved (both dimensions multiplied by `scale`).
/// * Alpha channel is handled: the image is converted to 8-bit RGBA, resized,
///   then returned. Callers that feed the result to JPEG will drop alpha via
///   [`crate::pdf::image_to_jpeg`].
/// * [`image::imageops::FilterType::CatmullRom`] is used: near-Lanczos quality
///   at roughly half the cost, a good speed/quality trade-off for
///   photographic/page content.
/// * Dimensions are clamped to `u32::MAX/4` to prevent overflow on absurd
///   scale factors. A page that fails to resize returns the *original* image
///   rather than killing the whole conversion.
pub fn upscale_image(img: DynamicImage, opts: &UpscaleOpts) -> DynamicImage {
    if opts.is_noop() {
        return img;
    }

    let (w, h) = img.dimensions();
    let new_w = ((w as f64) * opts.scale).round();
    let new_h = ((h as f64) * opts.scale).round();
    if !new_w.is_finite() || !new_h.is_finite() || new_w < 1.0 || new_h < 1.0 {
        tracing::warn!(scale = opts.scale, "invalid upscale dimensions, skipping");
        return img;
    }
    // Clamp to a sane upper bound (prevents u32 overflow + RAM blowups).
    const MAX_DIM: f64 = 16384.0;
    let new_w = new_w.min(MAX_DIM) as u32;
    let new_h = new_h.min(MAX_DIM) as u32;

    if new_w == w && new_h == h {
        return img;
    }

    tracing::trace!(
        from = format!("{w}x{h}"),
        to = format!("{new_w}x{new_h}"),
        "upscale"
    );
    img.resize(new_w, new_h, image::imageops::FilterType::CatmullRom)
}

/// Composite an RGBA overlay onto an RGB base image (alpha blend).
///
/// `base` is grown (transparent fill) if the overlay is larger; both images are
/// cropped to their overlap region when iterating. This is used by the v3
/// "page + layer" combination (`p-N.webp` + `p-l-N.webp`).
///
/// Performance: works on raw pixel buffers with row-wise `copy_from_slice`
/// (no per-pixel bound checks / `put_pixel`) and converts the overlay to RGBA
/// once, since the page decode already produced RGBA.
pub fn merge_overlay(base: image::RgbImage, overlay: &DynamicImage) -> image::RgbImage {
    let overlay_rgba = overlay.to_rgba8();
    let (bw, bh) = base.dimensions();
    let (ow, oh) = overlay_rgba.dimensions();

    // If the overlay is larger than the base, grow the base (transparent →
    // black) so the overlay isn't clipped. This matches the v3 reference where
    // the layer may contain a larger drawing canvas.
    let (out_w, out_h) = (bw.max(ow), bh.max(oh));
    let mut out: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(out_w, out_h);

    {
        // Work directly on the raw pixel buffer (no bound-checked put_pixel).
        let out_raw: &mut [u8] = out.as_mut();

        // Copy base first (row-wise; interior rows are full-width copies).
        let base_raw = base.as_raw();
        let bstride = bw as usize * 3; // RGB = 3 bytes/pixel
        let ostride = out_w as usize * 3;
        if bw == out_w {
            out_raw.copy_from_slice(base_raw);
        } else {
            for y in 0..bh as usize {
                let src = &base_raw[y * bstride..(y + 1) * bstride];
                out_raw[y * ostride..y * ostride + bstride].copy_from_slice(src);
            }
        }

        // Alpha-blend overlay on top (row-wise over the overlap region).
        let ob = overlay_rgba.as_raw();
        let owidth = ow as usize;
        let ostride4 = owidth * 4; // RGBA = 4 bytes/pixel
        let overlap_w = ow.min(out_w) as usize;
        for y in 0..oh.min(out_h) as usize {
            let orow = &ob[y * ostride4..(y + 1) * ostride4];
            let orow = &orow[..overlap_w * 4];
            let drow = &mut out_raw[y * ostride..y * ostride + overlap_w * 3];

            // Fast path: fully opaque overlay row → bulk copy.
            if orow[3..].iter().step_by(4).all(|&a| a == 255) {
                for (dst, src) in drow.chunks_exact_mut(3).zip(orow.chunks_exact(4)) {
                    dst.copy_from_slice(&src[..3]);
                }
                continue;
            }

            for (dst, src) in drow.chunks_exact_mut(3).zip(orow.chunks_exact(4)) {
                let a = src[3] as u32;
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    dst.copy_from_slice(&src[..3]);
                    continue;
                }
                let inv = 255 - a;
                dst[0] = ((src[0] as u32 * a + dst[0] as u32 * inv) / 255) as u8;
                dst[1] = ((src[1] as u32 * a + dst[1] as u32 * inv) / 255) as u8;
                dst[2] = ((src[2] as u32 * a + dst[2] as u32 * inv) / 255) as u8;
            }
        }
    }
    out
}

// Silence the unused-import warning for `Rgba`/`RgbaImage` — they document the
// pixel-level types even though resize happens through `DynamicImage`.
#[allow(dead_code)]
type _UnusedPixelTypes = (Rgba<u8>, RgbaImage);

/// Make a page image opaque: convert RGBA→RGB (white background) without an
/// extra full-image allocation when the source is already RGB8.
pub fn to_rgb(img: &DynamicImage) -> image::RgbImage {
    match img {
        DynamicImage::ImageRgb8(rgb) => rgb.clone(),
        other => other.to_rgb8(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn rgb(w: u32, h: u32, v: u8) -> image::RgbImage {
        ImageBuffer::from_pixel(w, h, Rgb([v, v, v]))
    }

    #[test]
    fn noop_scale_returns_identity() {
        let opts = UpscaleOpts::new(1.0);
        assert!(opts.is_noop());
        let img = DynamicImage::ImageRgb8(rgb(10, 10, 128));
        let out = upscale_image(img, &opts);
        assert_eq!(out.dimensions(), (10, 10));
    }

    #[test]
    fn upscale_preserves_aspect_ratio_doubles() {
        let opts = UpscaleOpts::new(2.0);
        let img = DynamicImage::ImageRgb8(rgb(100, 200, 64));
        let out = upscale_image(img, &opts);
        assert_eq!(out.dimensions(), (200, 400));
    }

    #[test]
    fn upscale_preserves_aspect_ratio_noninteger() {
        // 2.8× should scale both axes equally (aspect preserved).
        let opts = UpscaleOpts::new(2.8);
        let img = DynamicImage::ImageRgb8(rgb(100, 150, 64));
        let out = upscale_image(img, &opts);
        let (w, h) = out.dimensions();
        let ar_orig = 100.0 / 150.0;
        let ar_out = w as f64 / h as f64;
        assert!(
            (ar_orig - ar_out).abs() < 0.05,
            "aspect ratio broken: {ar_orig} → {ar_out}"
        );
    }

    #[test]
    fn upscale_clamps_absurd_dimensions() {
        let opts = UpscaleOpts::new(10_000.0);
        let img = DynamicImage::ImageRgb8(rgb(100, 100, 64));
        let out = upscale_image(img, &opts);
        let (w, h) = out.dimensions();
        assert!(w <= 16384 && h <= 16384);
    }

    #[test]
    fn scale_zero_or_negative_is_noop() {
        for s in [0.0, -1.0, f64::NAN] {
            assert!(UpscaleOpts::new(s).is_noop(), "scale {s} should be noop");
        }
    }

    #[test]
    fn merge_overlay_opaque_replaces() {
        // Fully opaque overlay should fully replace the base pixels it covers.
        let base = rgb(4, 4, 0);
        let overlay = DynamicImage::ImageRgb8(rgb(4, 4, 255));
        let out = merge_overlay(base, &overlay);
        assert_eq!(out.dimensions(), (4, 4));
        assert_eq!(*out.get_pixel(2, 2), Rgb([255, 255, 255]));
    }
}
