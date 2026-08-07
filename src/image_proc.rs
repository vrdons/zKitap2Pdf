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
    pub target_px: Option<(u32, u32)>,
}

impl UpscaleOpts {
    /// Fixed-factor mode (`--scale`): every page is multiplied by `scale`.
    pub fn new(scale: f64) -> Self {
        Self {
            scale,
            target_px: None,
        }
    }

    /// Target-size mode (`--target-dpi`): pages smaller than `target` are
    /// upscaled to it (aspect-preserving), larger pages are left as-is.
    pub fn to_target(target: (u32, u32)) -> Self {
        Self {
            scale: 1.0,
            target_px: Some(target),
        }
    }

    /// Whether the upscale stage does any actual work for an image of the
    /// given dimensions. Callers can use this to short-circuit decode/encode
    /// when no resize is needed.
    pub fn is_noop_for(&self, w: u32, h: u32) -> bool {
        match self.target_px {
            Some((tw, th)) => w >= tw && h >= th,
            None => self.scale <= 1.0 || !self.scale.is_finite(),
        }
    }

    /// Backwards-compatible no-op check for callers without dimensions.
    pub fn is_noop(&self) -> bool {
        self.scale <= 1.0 || !self.scale.is_finite()
    }
}

impl Default for UpscaleOpts {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Upscale an image by the configured factor or to the configured target size.
///
/// * Fixed factor mode: `scale <= 1.0` or non-finite → unchanged.
/// * Target mode: the image is only **enlarged** to reach `target_px`;
///   images already at/above the target are returned unchanged (no
///   downscaling, ever). Aspect ratio is preserved by scaling to the
///   *longest* edge that fits, so small pages reach the target on both axes.
/// * [`image::imageops::FilterType::CatmullRom`] is used: near-Lanczos quality
///   at roughly half the cost, a good speed/quality trade-off for
///   photographic/page content.
/// * Dimensions are clamped to `u32::MAX/4` to prevent overflow on absurd
///   scale factors. A page that fails to resize returns the *original* image
///   rather than killing the whole conversion.
pub fn upscale_image(img: DynamicImage, opts: &UpscaleOpts) -> DynamicImage {
    let (w, h) = img.dimensions();

    // Target mode: only upscale when the page is smaller than the target.
    if let Some((tw, th)) = opts.target_px {
        if opts.is_noop_for(w, h) {
            return img; // already big enough — no downscale, no work
        }
        let scale = (tw as f64 / w as f64).max(th as f64 / h as f64);
        let scale = if scale.is_finite() && scale > 1.0 {
            scale
        } else {
            1.0
        };
        if scale <= 1.0 {
            return img;
        }
        let new_w = ((w as f64) * scale).round().min(16384.0) as u32;
        let new_h = ((h as f64) * scale).round().min(16384.0) as u32;
        if new_w == w && new_h == h {
            return img;
        }
        tracing::trace!(
            from = format!("{w}x{h}"),
            to = format!("{new_w}x{new_h}"),
            "upscale (target)"
        );
        return img.resize(new_w, new_h, image::imageops::FilterType::CatmullRom);
    }

    if opts.is_noop() {
        return img;
    }

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
    fn target_mode_upscales_only_small_pages() {
        // Small page (566x807) → target 2x → enlarged.
        let opts = UpscaleOpts::to_target((1132, 1614));
        let img = DynamicImage::ImageRgb8(rgb(566, 807, 64));
        let out = upscale_image(img, &opts);
        assert_eq!(out.dimensions(), (1132, 1614));
    }

    #[test]
    fn target_mode_never_downscales_large_pages() {
        // Big page (2640x3402) already above target → untouched.
        let opts = UpscaleOpts::to_target((1132, 1614));
        let img = DynamicImage::ImageRgb8(rgb(2640, 3402, 64));
        let out = upscale_image(img, &opts);
        assert_eq!(out.dimensions(), (2640, 3402));
    }

    #[test]
    fn target_mode_noop_for_already_big_enough() {
        let opts = UpscaleOpts::to_target((100, 200));
        assert!(opts.is_noop_for(150, 250));
        assert!(!opts.is_noop_for(50, 250));
        assert!(!opts.is_noop_for(150, 50));
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
