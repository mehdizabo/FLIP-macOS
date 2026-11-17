use crate::core::policy::ext_lower;
use crate::engines::{bridge, EngineError};
use image::{GenericImageView, ImageFormat};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_MEGAPIXELS: u64 = 40_000_000;

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

fn normalize_to_png(
    source: &Path,
    staging_dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf, EngineError> {
    let tmp_png = staging_dir.join(format!("norm-{}.png", Uuid::new_v4().simple()));
    if bridge::convert_image(source, "PNG", &tmp_png, 1.0, false, true, Some(cancel)).is_ok() {
        return Ok(tmp_png);
    }
    ensure_png_via_rust_image(source, staging_dir)
}

pub fn convert(
    source: &Path,
    format: &str,
    output: &Path,
    staging_dir: &Path,
    quality_pct: u8,
    preserve_exif: bool,
    strip_gps_exif: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let fmt = format.trim().to_ascii_uppercase();
    check_dimensions_with_image(source)?;

    if fmt == "PDF" {
        let norm = normalize_to_png(source, staging_dir, cancel)?;
        bridge::convert_pdf_from_images(output, &[norm], Some(cancel))?;
        return Ok(());
    }

    if fmt == "WEBP" {
        let ext = ext_lower(source);
        // Normalize via ImageIO PNG first when possible so EXIF orientation + HEIC/AVIF are upright
        let readable = if ext == "webp" || ext == "ico" {
            source.to_path_buf()
        } else {
            normalize_to_png(source, staging_dir, cancel)?
        };
        let img = image::open(&readable).map_err(|e| {
            EngineError::ConversionFailed(format!("Failed to decode image for WebP: {e}"))
        })?;
        let (w, h) = img.dimensions();
        if (w as u64) * (h as u64) > MAX_MEGAPIXELS {
            return Err(EngineError::ConversionFailed(
                "Image exceeds 40 megapixel safety limit".to_string(),
            ));
        }
        img.save_with_format(output, ImageFormat::WebP).map_err(|e| {
            EngineError::ConversionFailed(format!("Failed to encode WebP: {e}"))
        })?;
        return Ok(());
    }

    let quality_01 = (quality_pct.clamp(10, 100) as f64) / 100.0;
    match bridge::convert_image(
        source,
        &fmt,
        output,
        quality_01,
        preserve_exif,
        strip_gps_exif,
        Some(cancel),
    ) {
        Ok(_) => Ok(()),
        Err(first_err) => {
            // Fallback for formats ImageIO may not decode (e.g. .ico or exotic .webp)
            if let Ok(png_fallback) = ensure_png_via_rust_image(source, staging_dir) {
                bridge::convert_image(
                    &png_fallback,
                    &fmt,
                    output,
                    quality_01,
                    preserve_exif,
                    strip_gps_exif,
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
