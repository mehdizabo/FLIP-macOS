use crate::core::policy::ext_lower;
use crate::engines::{bridge, EngineError};
use image::codecs::gif::{GifEncoder, Repeat};
use image::imageops::{replace, FilterType};
use image::{Delay, DynamicImage, Frame, GenericImageView, ImageBuffer, ImageFormat, Rgba};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_MEGAPIXELS: u64 = 40_000_000;
const ICO_MAX_SIDE: u32 = 256;
const ANIMATE_GIF_MAX_SIDE: u32 = 640;
const ANIMATE_GIF_DELAY_MS: u32 = 400;

fn check_dimensions_with_image(path: &Path) -> Result<(), EngineError> {
    if let Ok((w, h)) = image::image_dimensions(path) {
        if (w as u64) * (h as u64) > MAX_MEGAPIXELS {
            return Err(EngineError::ConversionFailed(
                "Image exceeds 40 megapixel safety limit".to_string(),
            ));
        }
    }
    Ok(())
}

fn ensure_png_via_rust_image(source: &Path, staging_dir: &Path) -> Result<PathBuf, EngineError> {
    let img = image::open(source)
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to decode image: {e}")))?;
    let (w, h) = img.dimensions();
    if (w as u64) * (h as u64) > MAX_MEGAPIXELS {
        return Err(EngineError::ConversionFailed(
            "Image exceeds 40 megapixel safety limit".to_string(),
        ));
    }
    let tmp_png = staging_dir.join(format!("norm-{}.png", Uuid::new_v4().simple()));
    img.save_with_format(&tmp_png, ImageFormat::Png)
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to write staging PNG: {e}")))?;
    Ok(tmp_png)
}

fn is_fatal_bridge_error(err: &bridge::BridgeError) -> bool {
    match err {
        bridge::BridgeError::Cancelled => true,
        bridge::BridgeError::CommandFailed(_, msg) => {
            msg.contains("40 megapixel") || msg.contains("zero dimensions") || msg.contains("zero size")
        }
        _ => false,
    }
}

fn normalize_to_png(
    source: &Path,
    staging_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, EngineError> {
    let tmp_png = staging_dir.join(format!("norm-{}.png", Uuid::new_v4().simple()));
    match bridge::convert_image(source, "PNG", &tmp_png, 1.0, false, true, Some(cancel)) {
        Ok(_) => Ok(tmp_png),
        Err(err) if is_fatal_bridge_error(&err) => Err(EngineError::Bridge(err)),
        Err(_) => ensure_png_via_rust_image(source, staging_dir),
    }
}

pub(crate) fn decode_via_bridge_or_rust(
    source: &Path,
    staging_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<DynamicImage, EngineError> {
    check_dimensions_with_image(source)?;
    let ext = ext_lower(source);
    let img = if ext == "ico" {
        match image::open(source) {
            Ok(decoded) => decoded,
            Err(_) => {
                let norm = normalize_to_png(source, staging_dir, cancel)?;
                image::open(&norm).map_err(|e| {
                    EngineError::ConversionFailed(format!("Failed to decode image: {e}"))
                })?
            }
        }
    } else {
        let readable = normalize_to_png(source, staging_dir, cancel)?;
        image::open(&readable).map_err(|e| {
            EngineError::ConversionFailed(format!("Failed to decode image: {e}"))
        })?
    };
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_MEGAPIXELS {
        return Err(EngineError::ConversionFailed(
            "Image exceeds 40 megapixel safety limit or has zero dimensions".to_string(),
        ));
    }
    Ok(img)
}

#[derive(Debug, Clone, Copy)]
pub struct ImageEncodeOptions {
    pub quality_pct: u8,
    pub preserve_exif: bool,
    pub strip_gps_exif: bool,
}

pub fn convert(
    source: &Path,
    format: &str,
    output: &Path,
    staging_dir: &Path,
    options: ImageEncodeOptions,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let fmt = format.trim().to_ascii_uppercase();
    check_dimensions_with_image(source)?;

    if fmt == "PDF" {
        let norm = normalize_to_png(source, staging_dir, cancel)?;
        bridge::convert_pdf_from_images(output, &[norm], Some(cancel))?;
        return Ok(());
    }

    if fmt == "TXT" {
        match bridge::ocr(source, Some(output), Some(cancel)) {
            Ok(_) => return Ok(()),
            Err(first_err) => {
                let no_text = match &first_err {
                    bridge::BridgeError::CommandFailed(_, msg) => {
                        msg.to_ascii_lowercase().contains("no readable text")
                    }
                    _ => false,
                };
                if is_fatal_bridge_error(&first_err) || no_text {
                    return Err(EngineError::Bridge(first_err));
                }
                if let Ok(png_fallback) = ensure_png_via_rust_image(source, staging_dir) {
                    bridge::ocr(&png_fallback, Some(output), Some(cancel))?;
                    return Ok(());
                }
                return Err(EngineError::Bridge(first_err));
            }
        }
    }

    if fmt == "REMOVE_BG" {
        match bridge::remove_bg(source, output, Some(cancel)) {
            Ok(_) => return Ok(()),
            Err(first_err) => {
                let no_subject = match &first_err {
                    bridge::BridgeError::CommandFailed(_, msg) => {
                        msg.to_ascii_lowercase().contains("no foreground subject")
                    }
                    _ => false,
                };
                if is_fatal_bridge_error(&first_err) || no_subject {
                    return Err(EngineError::Bridge(first_err));
                }
                if let Ok(png_fallback) = ensure_png_via_rust_image(source, staging_dir) {
                    bridge::remove_bg(&png_fallback, output, Some(cancel))?;
                    return Ok(());
                }
                return Err(EngineError::Bridge(first_err));
            }
        }
    }

    if fmt == "WEBP" {
        let img = decode_via_bridge_or_rust(source, staging_dir, cancel)?;
        img.save_with_format(output, ImageFormat::WebP).map_err(|e| {
            EngineError::ConversionFailed(format!("Failed to encode WebP: {e}"))
        })?;
        return Ok(());
    }

    if fmt == "ICO" {
        let img = decode_via_bridge_or_rust(source, staging_dir, cancel)?;
        let (w, h) = img.dimensions();
        let side = w.max(h).clamp(1, ICO_MAX_SIDE);
        let fitted = if w == side && h == side {
            img.to_rgba8()
        } else {
            let resized = if w > side || h > side {
                img.resize(side, side, FilterType::Lanczos3)
            } else {
                img
            };
            let rgba = resized.to_rgba8();
            let (rw, rh) = rgba.dimensions();
            let mut canvas: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_pixel(side, side, Rgba([0, 0, 0, 0]));
            let x_off = ((side.saturating_sub(rw)) / 2) as i64;
            let y_off = ((side.saturating_sub(rh)) / 2) as i64;
            replace(&mut canvas, &rgba, x_off, y_off);
            canvas
        };
        DynamicImage::ImageRgba8(fitted)
            .save_with_format(output, ImageFormat::Ico)
            .map_err(|e| EngineError::ConversionFailed(format!("Failed to encode ICO: {e}")))?;
        return Ok(());
    }

    let quality_01 = (options.quality_pct.clamp(10, 100) as f64) / 100.0;
    match bridge::convert_image(
        source,
        &fmt,
        output,
        quality_01,
        options.preserve_exif,
        options.strip_gps_exif,
        Some(cancel),
    ) {
        Ok(_) => Ok(()),
        Err(first_err) => {
            if is_fatal_bridge_error(&first_err) {
                return Err(EngineError::Bridge(first_err));
            }
            // Fallback for formats ImageIO may not decode (e.g. .ico or exotic .webp)
            if let Ok(png_fallback) = ensure_png_via_rust_image(source, staging_dir) {
                bridge::convert_image(
                    &png_fallback,
                    &fmt,
                    output,
                    quality_01,
                    options.preserve_exif,
                    options.strip_gps_exif,
                    Some(cancel),
                )?;
                Ok(())
            } else {
                Err(EngineError::Bridge(first_err))
            }
        }
    }
}

pub fn combine_into_pdf(
    sources: &[PathBuf],
    output_pdf: &Path,
    staging_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let mut prepared = Vec::with_capacity(sources.len());
    for src in sources {
        check_dimensions_with_image(src)?;
        let norm = normalize_to_png(src, staging_dir, cancel)?;
        prepared.push(norm);
    }
    bridge::convert_pdf_from_images(output_pdf, &prepared, Some(cancel))?;
    Ok(())
}

pub fn animate_into_gif(
    sources: &[PathBuf],
    output_gif: &Path,
    staging_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    if sources.len() < 2 {
        return Err(EngineError::Unsupported(
            "Select at least 2 images to animate into GIF.".to_string(),
        ));
    }

    let mut decoded = Vec::with_capacity(sources.len());
    let mut max_w: u32 = 1;
    let mut max_h: u32 = 1;

    for src in sources {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let img = decode_via_bridge_or_rust(src, staging_dir, cancel)?;
        let (w, h) = img.dimensions();
        let bounded = if w > ANIMATE_GIF_MAX_SIDE || h > ANIMATE_GIF_MAX_SIDE {
            img.resize(ANIMATE_GIF_MAX_SIDE, ANIMATE_GIF_MAX_SIDE, FilterType::Lanczos3)
        } else {
            img
        };
        let (bw, bh) = bounded.dimensions();
        max_w = max_w.max(bw);
        max_h = max_h.max(bh);
        decoded.push(bounded);
    }

    let canvas_w = max_w.clamp(1, ANIMATE_GIF_MAX_SIDE);
    let canvas_h = max_h.clamp(1, ANIMATE_GIF_MAX_SIDE);

    let file = File::create(output_gif)?;
    let mut encoder = GifEncoder::new_with_speed(file, 10);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to configure GIF repeat: {e}")))?;

    let delay = Delay::from_numer_denom_ms(ANIMATE_GIF_DELAY_MS, 1);
    for img in decoded {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let (fw, fh) = img.dimensions();
        let fitted = if fw > canvas_w || fh > canvas_h {
            img.resize(canvas_w, canvas_h, FilterType::Lanczos3)
        } else {
            img
        };
        let rgba = fitted.to_rgba8();
        let (rw, rh) = rgba.dimensions();
        let mut canvas: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(canvas_w, canvas_h, Rgba([0, 0, 0, 0]));
        let x_off = ((canvas_w.saturating_sub(rw)) / 2) as i64;
        let y_off = ((canvas_h.saturating_sub(rh)) / 2) as i64;
        replace(&mut canvas, &rgba, x_off, y_off);

        let frame = Frame::from_parts(canvas, 0, 0, delay);
        encoder
            .encode_frame(frame)
            .map_err(|e| EngineError::ConversionFailed(format!("Failed to encode GIF frame: {e}")))?;
    }

    Ok(())
}
