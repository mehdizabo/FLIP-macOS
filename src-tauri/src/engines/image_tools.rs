//! Image tools ("Strumenti"): crop, rotate/flip, scale, palette, metadata removal and redaction.
//!
//! Tools are ordinary action tokens (`CROP_9_16`, `ROTATE_RIGHT`, `SCALE_1080P`, `PALETTE`,
//! `STRIP_EXIF`, ...) that flow through the same job pipeline as format conversions: they stage their
//! output, never touch the original and never overwrite an existing file. Unlike a format they keep
//! the source's own format (a rotated JPG stays a JPG), so the output format is resolved from the
//! source extension and the file gets a descriptive suffix (`photo-crop-9x16.jpg`).

use crate::core::naming::format_extension;
use crate::core::policy::{ext_lower, IMAGE_EXTS};
use crate::core::preferences::AppPreferences;
use crate::core::staging::JobStaging;
use crate::engines::image::decode_via_bridge_or_rust;
use crate::engines::redact::{self, Region};
use crate::engines::{bridge, resolve_target_dir, validate_container, validate_output, EngineError};
use ::image::codecs::jpeg::JpegEncoder;
use ::image::imageops::FilterType;
use ::image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, Rgb, RgbImage, Rgba};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Target size for the `SCALE_1MB` preset ("under 1 MB").
const ONE_MB_BYTES: u64 = 1_000_000;
/// Largest file the lossless metadata stripper will read into memory.
const MAX_STRIP_BYTES: u64 = 256 * 1024 * 1024;
const PALETTE_COLORS: usize = 6;
const PALETTE_SAMPLE_SIDE: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropRatio {
    Portrait9x16,
    Portrait4x5,
    Square,
    Landscape16x9,
}

impl CropRatio {
    fn dims(self) -> (u32, u32) {
        match self {
            Self::Portrait9x16 => (9, 16),
            Self::Portrait4x5 => (4, 5),
            Self::Square => (1, 1),
            Self::Landscape16x9 => (16, 9),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Percent(u32),
    /// Shorter side becomes this many pixels (never upscales): 4032x3024 at 1080 -> 1440x1080.
    ShorterSide(u32),
    /// Shrinks (quality first for JPEG, then dimensions) until the file is under the limit.
    MaxBytes(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageTool {
    Crop(CropRatio),
    RotateRight,
    RotateLeft,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
    Scale(Scale),
    Redact,
    Palette,
    StripMetadata,
}

/// Every tool token, in the order the wheel presents them.
pub const ALL_TOOL_TOKENS: &[&str] = &[
    "CROP_9_16",
    "CROP_4_5",
    "CROP_1_1",
    "CROP_16_9",
    "ROTATE_RIGHT",
    "ROTATE_LEFT",
    "ROTATE_180",
    "FLIP_H",
    "FLIP_V",
    "SCALE_50",
    "SCALE_25",
    "SCALE_1080P",
    "SCALE_720P",
    "SCALE_1MB",
    "REDACT",
    "PALETTE",
    "STRIP_EXIF",
];

impl ImageTool {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_uppercase().as_str() {
            "CROP_9_16" => Self::Crop(CropRatio::Portrait9x16),
            "CROP_4_5" => Self::Crop(CropRatio::Portrait4x5),
            "CROP_1_1" => Self::Crop(CropRatio::Square),
            "CROP_16_9" => Self::Crop(CropRatio::Landscape16x9),
            "ROTATE_RIGHT" => Self::RotateRight,
            "ROTATE_LEFT" => Self::RotateLeft,
            "ROTATE_180" => Self::Rotate180,
            "FLIP_H" => Self::FlipHorizontal,
            "FLIP_V" => Self::FlipVertical,
            "SCALE_50" => Self::Scale(Scale::Percent(50)),
            "SCALE_25" => Self::Scale(Scale::Percent(25)),
            "SCALE_1080P" => Self::Scale(Scale::ShorterSide(1080)),
            "SCALE_720P" => Self::Scale(Scale::ShorterSide(720)),
            "SCALE_1MB" => Self::Scale(Scale::MaxBytes(ONE_MB_BYTES)),
            "REDACT" => Self::Redact,
            "PALETTE" => Self::Palette,
            "STRIP_EXIF" => Self::StripMetadata,
            _ => return None,
        })
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Crop(CropRatio::Portrait9x16) => "CROP_9_16",
            Self::Crop(CropRatio::Portrait4x5) => "CROP_4_5",
            Self::Crop(CropRatio::Square) => "CROP_1_1",
            Self::Crop(CropRatio::Landscape16x9) => "CROP_16_9",
            Self::RotateRight => "ROTATE_RIGHT",
            Self::RotateLeft => "ROTATE_LEFT",
            Self::Rotate180 => "ROTATE_180",
            Self::FlipHorizontal => "FLIP_H",
            Self::FlipVertical => "FLIP_V",
            Self::Scale(Scale::Percent(50)) => "SCALE_50",
            Self::Scale(Scale::Percent(_)) => "SCALE_25",
            Self::Scale(Scale::ShorterSide(1080)) => "SCALE_1080P",
            Self::Scale(Scale::ShorterSide(_)) => "SCALE_720P",
            Self::Scale(Scale::MaxBytes(_)) => "SCALE_1MB",
            Self::Redact => "REDACT",
            Self::Palette => "PALETTE",
            Self::StripMetadata => "STRIP_EXIF",
        }
    }

    /// Descriptive file-name suffix, e.g. `photo-crop-9x16.jpg`.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Crop(CropRatio::Portrait9x16) => "-crop-9x16",
            Self::Crop(CropRatio::Portrait4x5) => "-crop-4x5",
            Self::Crop(CropRatio::Square) => "-crop-1x1",
            Self::Crop(CropRatio::Landscape16x9) => "-crop-16x9",
            Self::RotateRight => "-rotate-right",
            Self::RotateLeft => "-rotate-left",
            Self::Rotate180 => "-rotate-180",
            Self::FlipHorizontal => "-flip-h",
            Self::FlipVertical => "-flip-v",
            Self::Scale(Scale::Percent(50)) => "-50pct",
            Self::Scale(Scale::Percent(_)) => "-25pct",
            Self::Scale(Scale::ShorterSide(1080)) => "-1080p",
            Self::Scale(Scale::ShorterSide(_)) => "-720p",
            Self::Scale(Scale::MaxBytes(_)) => "-under-1mb",
            Self::Redact => "-redacted",
            Self::Palette => "-palette",
            Self::StripMetadata => "-clean",
        }
    }

    /// Format of the produced file for a given source extension.
    pub fn output_format(self, source_ext: &str) -> &'static str {
        if self == Self::Palette {
            return "PNG";
        }
        match source_ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "heic" | "heif" | "avif" => "JPG",
            "webp" => "WEBP",
            "bmp" => "BMP",
            "tif" | "tiff" => "TIFF",
            // PNG stays PNG; GIF/ICO/ICNS/SVG have no sensible same-format round trip here.
            _ => "PNG",
        }
    }
}

fn carries_metadata(ext: &str) -> bool {
    matches!(
        ext,
        "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff" | "heic" | "heif" | "avif"
    )
}

/// Tool tokens that apply to a source extension (empty for non-images).
pub fn tools_for_ext(ext: &str) -> Vec<&'static str> {
    let ext = ext.to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return Vec::new();
    }
    ALL_TOOL_TOKENS
        .iter()
        .copied()
        .filter(|t| *t != "STRIP_EXIF" || carries_metadata(&ext))
        .collect()
}

/// Tool tokens that apply to *every* selected file (empty as soon as one file is not an image).
pub fn tools_for_paths<P: AsRef<Path>>(paths: &[P]) -> Vec<&'static str> {
    let mut shared: Option<Vec<&'static str>> = None;
    for path in paths {
        let tools = tools_for_ext(&ext_lower(path.as_ref()));
        shared = Some(match shared {
            None => tools,
            Some(prev) => prev.into_iter().filter(|t| tools.contains(t)).collect(),
        });
    }
    shared.unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Pixel geometry
// ---------------------------------------------------------------------------------------------

/// Largest centred `rw:rh` rectangle inside the image.
pub fn crop_center(img: &DynamicImage, ratio: CropRatio) -> DynamicImage {
    let (rw, rh) = ratio.dims();
    let (w, h) = img.dimensions();
    let (cw, ch) = if u64::from(w) * u64::from(rh) >= u64::from(h) * u64::from(rw) {
        // Wider than the target ratio: keep the full height, trim the sides.
        let cw = (u64::from(h) * u64::from(rw) / u64::from(rh)).max(1) as u32;
        (cw.min(w), h)
    } else {
        let ch = (u64::from(w) * u64::from(rh) / u64::from(rw)).max(1) as u32;
        (w, ch.min(h))
    };
    img.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch)
}

/// Dimensions after a scale preset that is defined purely by geometry.
pub fn scaled_dimensions(w: u32, h: u32, scale: Scale) -> (u32, u32) {
    match scale {
        Scale::Percent(p) => (
            ((u64::from(w) * u64::from(p)) / 100).max(1) as u32,
            ((u64::from(h) * u64::from(p)) / 100).max(1) as u32,
        ),
        Scale::ShorterSide(n) => {
            let shorter = w.min(h);
            if shorter <= n {
                return (w, h);
            }
            let f = f64::from(n) / f64::from(shorter);
            (
                ((f64::from(w) * f).round() as u32).max(1),
                ((f64::from(h) * f).round() as u32).max(1),
            )
        }
        Scale::MaxBytes(_) => (w, h),
    }
}

// ---------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------

fn encode_to(path: &Path, img: &DynamicImage, format: &str, quality: u8) -> Result<(), EngineError> {
    let map_err =
        |e: ::image::ImageError| EngineError::ConversionFailed(format!("Failed to encode {format}: {e}"));
    match format {
        "JPG" => {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            let encoder = JpegEncoder::new_with_quality(&mut writer, quality.clamp(10, 100));
            DynamicImage::ImageRgb8(img.to_rgb8())
                .write_with_encoder(encoder)
                .map_err(map_err)
        }
        "PNG" => img.save_with_format(path, ImageFormat::Png).map_err(map_err),
        "WEBP" => img.save_with_format(path, ImageFormat::WebP).map_err(map_err),
        "TIFF" => img.save_with_format(path, ImageFormat::Tiff).map_err(map_err),
        "BMP" => img.save_with_format(path, ImageFormat::Bmp).map_err(map_err),
        other => Err(EngineError::Unsupported(format!(
            "Image tools cannot write {other}."
        ))),
    }
}

/// Encodes until the file fits `limit` bytes: JPEG quality first (down to 60), then dimensions.
fn encode_under(
    path: &Path,
    img: &DynamicImage,
    format: &str,
    quality: u8,
    limit: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let mut current = img.clone();
    let mut q = quality.clamp(10, 100);
    for _ in 0..16 {
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        encode_to(path, &current, format, q)?;
        let size = fs::metadata(path)?.len();
        if size <= limit {
            return Ok(());
        }
        if format == "JPG" && q > 60 {
            q = q.saturating_sub(12).max(60);
            continue;
        }
        let factor = ((limit as f64 / size as f64).sqrt() * 0.92).clamp(0.3, 0.95);
        let (w, h) = current.dimensions();
        let (nw, nh) = (
            ((f64::from(w) * factor) as u32).max(1),
            ((f64::from(h) * factor) as u32).max(1),
        );
        if nw.min(nh) < 32 {
            break;
        }
        current = current.resize_exact(nw, nh, FilterType::Lanczos3);
    }
    Err(EngineError::ConversionFailed(
        "Could not reduce the image below 1 MB without making it unusably small".to_string(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------------------------

/// Dominant colours by median cut, most frequent first.
pub fn dominant_colors(img: &DynamicImage, count: usize) -> Vec<([u8; 3], usize)> {
    let (w, h) = img.dimensions();
    let small = if w.max(h) > PALETTE_SAMPLE_SIDE {
        img.thumbnail(PALETTE_SAMPLE_SIDE, PALETTE_SAMPLE_SIDE)
    } else {
        img.clone()
    };
    let rgba = small.to_rgba8();
    let pixels: Vec<[u8; 3]> = rgba
        .pixels()
        .filter(|p| p.0[3] >= 128)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect();
    if pixels.is_empty() {
        return Vec::new();
    }

    let mut boxes: Vec<Vec<[u8; 3]>> = vec![pixels.clone()];
    while boxes.len() < count {
        // Split the box with the widest channel range that still has more than one pixel.
        let mut best: Option<(usize, usize, u8)> = None;
        for (i, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            for ch in 0..3 {
                let (lo, hi) = b
                    .iter()
                    .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[ch]), hi.max(p[ch])));
                let range = hi - lo;
                if range > 0 && best.is_none_or(|(_, _, r)| range > r) {
                    best = Some((i, ch, range));
                }
            }
        }
        let Some((idx, ch, _)) = best else { break };
        let mut b = boxes.swap_remove(idx);
        b.sort_unstable_by_key(|p| p[ch]);
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }

    // Median cut can straddle equally sized colour clusters (e.g. flat green/white/red areas) and
    // average two of them into a muddy swatch, so its boxes only seed a few k-means iterations.
    let seeds: Vec<[f64; 3]> = boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let n = b.len() as f64;
            let sum = b.iter().fold([0f64; 3], |mut acc, p| {
                for c in 0..3 {
                    acc[c] += f64::from(p[c]);
                }
                acc
            });
            [sum[0] / n, sum[1] / n, sum[2] / n]
        })
        .collect();
    merge_close_colors(refine_clusters(&pixels, seeds))
}

fn color_distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|c| (a[c] - b[c]) * (a[c] - b[c])).sum()
}

/// A few Lloyd iterations from deterministic seeds; returns (colour, pixel count) per non-empty cluster.
fn refine_clusters(pixels: &[[u8; 3]], mut centers: Vec<[f64; 3]>) -> Vec<([u8; 3], usize)> {
    let nearest = |centers: &[[f64; 3]], p: [u8; 3]| -> usize {
        let pf = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        (0..centers.len())
            .min_by(|&a, &b| {
                color_distance_sq(centers[a], pf)
                    .partial_cmp(&color_distance_sq(centers[b], pf))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0)
    };
    let mut counts = vec![0usize; centers.len()];
    for _ in 0..8 {
        let mut sums = vec![[0f64; 3]; centers.len()];
        counts = vec![0usize; centers.len()];
        for &p in pixels {
            let i = nearest(&centers, p);
            counts[i] += 1;
            for c in 0..3 {
                sums[i][c] += f64::from(p[c]);
            }
        }
        let mut moved = false;
        for i in 0..centers.len() {
            if counts[i] == 0 {
                continue;
            }
            let n = counts[i] as f64;
            let next = [sums[i][0] / n, sums[i][1] / n, sums[i][2] / n];
            if color_distance_sq(next, centers[i]) > 0.25 {
                moved = true;
            }
            centers[i] = next;
        }
        if !moved {
            break;
        }
    }
    centers
        .iter()
        .zip(&counts)
        .filter(|(_, &n)| n > 0)
        .map(|(c, &n)| {
            (
                [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8],
                n,
            )
        })
        .collect()
}

/// Folds swatches that are visually the same colour into the more frequent one.
fn merge_close_colors(mut colors: Vec<([u8; 3], usize)>) -> Vec<([u8; 3], usize)> {
    const MERGE_DISTANCE: f64 = 28.0;
    colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut kept: Vec<([u8; 3], usize)> = Vec::with_capacity(colors.len());
    for (rgb, n) in colors {
        let f = [f64::from(rgb[0]), f64::from(rgb[1]), f64::from(rgb[2])];
        let near = kept.iter_mut().find(|(k, _)| {
            color_distance_sq([f64::from(k[0]), f64::from(k[1]), f64::from(k[2])], f)
                <= MERGE_DISTANCE * MERGE_DISTANCE
        });
        match near {
            Some((_, total)) => *total += n,
            None => kept.push((rgb, n)),
        }
    }
    kept.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    kept
}

pub fn hex_label(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// 5x7 glyphs for `#` and hexadecimal digits (one row per byte, bit 4 = leftmost pixel).
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        _ => [0; 7],
    }
}

fn draw_text(img: &mut RgbImage, text: &str, x: u32, y: u32, scale: u32, ink: Rgb<u8>) {
    let advance = 6 * scale;
    for (i, ch) in text.chars().enumerate() {
        let rows = glyph(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = x + i as u32 * advance + col * scale + dx;
                        let py = y + row as u32 * scale + dy;
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, ink);
                        }
                    }
                }
            }
        }
    }
}

/// Swatch strip: one colour block per dominant colour with its hex code underneath.
pub fn render_palette(colors: &[([u8; 3], usize)]) -> RgbImage {
    const SWATCH: u32 = 160;
    const LABEL_H: u32 = 44;
    let n = colors.len().max(1) as u32;
    let mut img: RgbImage = ImageBuffer::from_pixel(n * SWATCH, SWATCH + LABEL_H, Rgb([250, 250, 250]));
    for (i, (rgb, _)) in colors.iter().enumerate() {
        let x0 = i as u32 * SWATCH;
        for y in 0..SWATCH {
            for x in 0..SWATCH {
                img.put_pixel(x0 + x, y, Rgb(*rgb));
            }
        }
        let label = hex_label(*rgb);
        let text_w = label.chars().count() as u32 * 18 - 3;
        draw_text(
            &mut img,
            &label,
            x0 + (SWATCH.saturating_sub(text_w)) / 2,
            SWATCH + 12,
            3,
            Rgb([24, 24, 24]),
        );
    }
    img
}

// ---------------------------------------------------------------------------------------------
// Metadata removal
// ---------------------------------------------------------------------------------------------

/// EXIF orientation (1..=8) of a JPEG, if present.
pub fn jpeg_exif_orientation(data: &[u8]) -> Option<u16> {
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 || marker == 0xFF {
            i += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        let seg_end = i + 2 + len;
        if len < 2 || seg_end > data.len() {
            return None;
        }
        let body = &data[i + 4..seg_end];
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return tiff_orientation(&body[6..]);
        }
        i = seg_end;
    }
    None
}

fn tiff_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let rd16 = |o: usize| -> Option<u16> {
        let b = tiff.get(o..o + 2)?;
        Some(if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let rd32 = |o: usize| -> Option<u32> {
        let b = tiff.get(o..o + 4)?;
        Some(if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = rd32(4)? as usize;
    let entries = usize::from(rd16(ifd)?);
    for n in 0..entries {
        let e = ifd + 2 + n * 12;
        if rd16(e)? == 0x0112 {
            return rd16(e + 8);
        }
    }
    None
}

/// Removes EXIF/XMP/IPTC/comment segments from a JPEG without re-encoding (ICC profile kept).
pub fn strip_jpeg(data: &[u8]) -> Result<Vec<u8>, EngineError> {
    let bad = || EngineError::ConversionFailed("Not a valid JPEG file".to_string());
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut i = 2usize;
    while i < data.len() {
        if data[i] != 0xFF {
            return Err(bad());
        }
        let marker = *data.get(i + 1).ok_or_else(bad)?;
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xD9 {
            out.extend_from_slice(&data[i..]);
            return Ok(out);
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 || marker == 0xD8 {
            out.extend_from_slice(&data[i..i + 2]);
            i += 2;
            continue;
        }
        let len_bytes = data.get(i + 2..i + 4).ok_or_else(bad)?;
        let len = usize::from(u16::from_be_bytes([len_bytes[0], len_bytes[1]]));
        let seg_end = i + 2 + len;
        if len < 2 || seg_end > data.len() {
            return Err(bad());
        }
        // APP0 (JFIF), APP2 (ICC profile) and APP14 (Adobe colour transform) are needed to render.
        let keep = !matches!(marker, 0xE1 | 0xE3..=0xED | 0xFE);
        if keep {
            out.extend_from_slice(&data[i..seg_end]);
        }
        if marker == 0xDA {
            // Entropy-coded scan data: copy everything that follows verbatim.
            out.extend_from_slice(&data[seg_end..]);
            return Ok(out);
        }
        i = seg_end;
    }
    Ok(out)
}

/// Removes EXIF/text/time chunks from a PNG without re-encoding.
pub fn strip_png(data: &[u8]) -> Result<Vec<u8>, EngineError> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let bad = || EngineError::ConversionFailed("Not a valid PNG file".to_string());
    if data.len() < 8 || &data[..8] != SIG {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(SIG);
    let mut i = 8usize;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > data.len() {
            return Err(bad());
        }
        let kind = &data[i + 4..i + 8];
        if !matches!(kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME") {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    Ok(out)
}

/// Removes EXIF and XMP chunks from a WebP container without re-encoding.
pub fn strip_webp(data: &[u8]) -> Result<Vec<u8>, EngineError> {
    let bad = || EngineError::ConversionFailed("Not a valid WebP file".to_string());
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..12]);
    let mut i = 12usize;
    while i + 8 <= data.len() {
        let kind = &data[i..i + 4];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let padded = len + (len & 1);
        let end = i + 8 + padded;
        if end > data.len() {
            return Err(bad());
        }
        match kind {
            b"EXIF" | b"XMP " => {}
            b"VP8X" if len >= 10 => {
                let start = out.len();
                out.extend_from_slice(&data[i..end]);
                // Flags byte: clear the EXIF (0x08) and XMP (0x04) presence bits.
                out[start + 8] &= !0x0C;
            }
            _ => out.extend_from_slice(&data[i..end]),
        }
        i = end;
    }
    let riff_len = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Redaction ("Oscura")
// ---------------------------------------------------------------------------------------------

/// Covers every region with opaque black, padded by `max(2 px, 0.3 × box height)` per side and
/// clamped to the image. Only the box pixels are touched, so the cost is the covered area.
pub fn paint_regions(img: &DynamicImage, regions: &[Region]) -> DynamicImage {
    if img.color().has_alpha() {
        let mut out = img.to_rgba8();
        for (x0, y0, x1, y1) in region_rects(regions, out.width(), out.height()) {
            for y in y0..y1 {
                for x in x0..x1 {
                    out.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }
        DynamicImage::ImageRgba8(out)
    } else {
        let mut out = img.to_rgb8();
        for (x0, y0, x1, y1) in region_rects(regions, out.width(), out.height()) {
            for y in y0..y1 {
                for x in x0..x1 {
                    out.put_pixel(x, y, Rgb([0, 0, 0]));
                }
            }
        }
        DynamicImage::ImageRgb8(out)
    }
}

/// Pixel rectangles `(x0, y0, x1, y1)` of the padded regions, right/bottom exclusive, clamped to the
/// image bounds.
fn region_rects(regions: &[Region], w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    let wf = f64::from(w);
    let hf = f64::from(h);
    regions
        .iter()
        .map(|region| {
            let left = region.x * wf;
            let top = region.y * hf;
            let right = (region.x + region.w) * wf;
            let bottom = (region.y + region.h) * hf;
            let pad = 2.0_f64.max(0.3 * (bottom - top));
            (
                (left - pad).floor().clamp(0.0, wf) as u32,
                (top - pad).floor().clamp(0.0, hf) as u32,
                (right + pad).ceil().clamp(0.0, wf) as u32,
                (bottom + pad).ceil().clamp(0.0, hf) as u32,
            )
        })
        .collect()
}

/// The bridge reports boxes for the upright image; covering a differently-shaped image would put the
/// black boxes in the wrong place, so a mismatch is refused rather than guessed.
fn ensure_same_aspect(img_w: u32, img_h: u32, ocr_w: u32, ocr_h: u32) -> Result<(), EngineError> {
    let valid = img_w > 0 && img_h > 0 && ocr_w > 0 && ocr_h > 0;
    let close = valid && {
        let img_ratio = f64::from(img_w) / f64::from(img_h);
        let ocr_ratio = f64::from(ocr_w) / f64::from(ocr_h);
        ((img_ratio - ocr_ratio) / img_ratio.max(ocr_ratio)).abs() <= 0.01
    };
    if close {
        return Ok(());
    }
    Err(EngineError::ConversionFailed(format!(
        "OCR boxes are {ocr_w}x{ocr_h} but the image is {img_w}x{img_h}: refusing to cover the wrong areas"
    )))
}

fn redact_image(
    img: &DynamicImage,
    source: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<DynamicImage, EngineError> {
    let value = bridge::ocr_boxes(source, Some(cancel))?;
    let ocr: redact::OcrBoxes = serde_json::from_value(value).map_err(|e| {
        EngineError::ConversionFailed(format!("Unreadable OCR result from the bridge: {e}"))
    })?;
    let (img_w, img_h) = img.dimensions();
    ensure_same_aspect(img_w, img_h, ocr.width, ocr.height)?;
    let regions = redact::find_sensitive(&ocr);
    if regions.is_empty() {
        return Err(EngineError::ConversionFailed(
            "No sensitive data was recognised. Nothing was written: check the image yourself before sharing it."
                .to_string(),
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    Ok(paint_regions(img, &regions))
}

// ---------------------------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------------------------

fn strip_metadata(
    source: &Path,
    out: &Path,
    staging_dir: &Path,
    quality: u8,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let ext = ext_lower(source);
    if matches!(ext.as_str(), "heic" | "heif" | "avif") {
        // ImageIO decodes, bakes orientation and writes a JPEG without the source's metadata.
        let q = f64::from(quality.clamp(10, 100)) / 100.0;
        bridge::convert_image(source, "JPG", out, q, false, true, Some(cancel))?;
        return Ok(());
    }

    if fs::metadata(source)?.len() > MAX_STRIP_BYTES {
        return Err(EngineError::ConversionFailed(
            "File is too large to strip metadata safely".to_string(),
        ));
    }
    let data = fs::read(source)?;
    let lossless = match ext.as_str() {
        "jpg" | "jpeg" => {
            // A non-default EXIF orientation is part of how the picture is displayed: removing the
            // tag alone would leave it sideways, so those files take the pixel path (orientation
            // baked in, then written without metadata).
            if jpeg_exif_orientation(&data).is_some_and(|o| o != 1) {
                None
            } else {
                Some(strip_jpeg(&data)?)
            }
        }
        "png" => Some(strip_png(&data)?),
        "webp" => Some(strip_webp(&data)?),
        _ => None,
    };
    if let Some(bytes) = lossless {
        fs::write(out, bytes)?;
        return Ok(());
    }
    let img = decode_via_bridge_or_rust(source, staging_dir, cancel)?;
    encode_to(out, &img, ImageTool::StripMetadata.output_format(&ext), quality)
}

/// Runs `tool` on `source` and writes the result to `out`.
pub fn apply(
    tool: ImageTool,
    source: &Path,
    out: &Path,
    staging_dir: &Path,
    quality: u8,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let ext = ext_lower(source);
    if !tools_for_ext(&ext).contains(&tool.token()) {
        return Err(EngineError::Unsupported(format!(
            "{} is not available for .{ext} images.",
            tool.token()
        )));
    }
    let format = tool.output_format(&ext);

    if tool == ImageTool::StripMetadata {
        return strip_metadata(source, out, staging_dir, quality, cancel);
    }

    let img = decode_via_bridge_or_rust(source, staging_dir, cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    let result = match tool {
        ImageTool::Crop(ratio) => crop_center(&img, ratio),
        ImageTool::RotateRight => img.rotate90(),
        ImageTool::RotateLeft => img.rotate270(),
        ImageTool::Rotate180 => img.rotate180(),
        ImageTool::FlipHorizontal => img.fliph(),
        ImageTool::FlipVertical => img.flipv(),
        ImageTool::Scale(Scale::MaxBytes(limit)) => {
            return encode_under(out, &img, format, quality, limit, cancel);
        }
        ImageTool::Scale(scale) => {
            let (w, h) = img.dimensions();
            let (nw, nh) = scaled_dimensions(w, h, scale);
            if (nw, nh) == (w, h) {
                img
            } else {
                img.resize_exact(nw, nh, FilterType::Lanczos3)
            }
        }
        ImageTool::Redact => redact_image(&img, source, cancel)?,
        ImageTool::Palette => {
            let colors = dominant_colors(&img, PALETTE_COLORS);
            if colors.is_empty() {
                return Err(EngineError::ConversionFailed(
                    "The image has no visible pixels to build a palette from".to_string(),
                ));
            }
            DynamicImage::ImageRgb8(render_palette(&colors))
        }
        ImageTool::StripMetadata => unreachable!("handled above"),
    };
    encode_to(out, &result, format, quality)
}

/// Crops `src` to a normalized `(norm_x, norm_y, norm_w, norm_h)` rectangle and writes `<stem>-cropped.<ext>`
/// using atomic staging and non-overwriting destination naming.
pub fn apply_manual_crop(
    src: &Path,
    norm_x: f64,
    norm_y: f64,
    norm_w: f64,
    norm_h: f64,
    prefs: &AppPreferences,
    cancel: &AtomicBool,
) -> Result<PathBuf, EngineError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    if !norm_x.is_finite()
        || !norm_y.is_finite()
        || !norm_w.is_finite()
        || !norm_h.is_finite()
        || norm_w <= 0.01
        || norm_h <= 0.01
    {
        return Err(EngineError::ConversionFailed(
            "Manual crop selection must be wider and taller than 1% of the image.".to_string(),
        ));
    }

    let nx = norm_x.clamp(0.0, 1.0);
    let ny = norm_y.clamp(0.0, 1.0);
    let nw = norm_w.clamp(0.0, 1.0 - nx);
    let nh = norm_h.clamp(0.0, 1.0 - ny);
    if nw <= 0.01 || nh <= 0.01 {
        return Err(EngineError::ConversionFailed(
            "Manual crop selection after clamping is too small.".to_string(),
        ));
    }

    let ext = ext_lower(src);
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return Err(EngineError::Unsupported(format!(
            "Manual crop is not available for .{ext} images."
        )));
    }

    validate_container(src)?;
    let target_dir = resolve_target_dir(src, prefs)?;
    let staging = JobStaging::create(&target_dir)?;
    let out_format = ImageTool::Crop(CropRatio::Square).output_format(&ext);
    let staged = staging.staged_path(&format!("output.{}", format_extension(out_format)));

    let cancel_arc = Arc::new(AtomicBool::new(cancel.load(Ordering::Relaxed)));
    let img = decode_via_bridge_or_rust(src, staging.dir(), &cancel_arc)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }

    let (w, h) = img.dimensions();
    let px = ((nx * f64::from(w)).round() as u32).min(w.saturating_sub(1));
    let py = ((ny * f64::from(h)).round() as u32).min(h.saturating_sub(1));
    let max_pw = w.saturating_sub(px).max(1);
    let max_ph = h.saturating_sub(py).max(1);
    let pw = ((nw * f64::from(w)).round() as u32).clamp(1, max_pw);
    let ph = ((nh * f64::from(h)).round() as u32).clamp(1, max_ph);
    let cropped = img.crop_imm(px, py, pw, ph);

    encode_to(&staged, &cropped, out_format, prefs.image_quality)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EngineError::Cancelled);
    }
    validate_output(&staged, out_format)?;
    let committed = staging.commit_one(&staged, src, Some("-cropped"), out_format)?;
    staging.cleanup()?;
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::redact::SensitiveKind;
    use ::image::Rgba;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(w, h, Rgba(rgba)))
    }

    fn region(x: f64, y: f64, w: f64, h: f64) -> Region {
        Region { kind: SensitiveKind::Iban, x, y, w, h }
    }

    #[test]
    fn every_token_round_trips_through_parse_token_and_has_a_unique_suffix() {
        let mut suffixes = std::collections::HashSet::new();
        for token in ALL_TOOL_TOKENS {
            let tool = ImageTool::parse(token).unwrap_or_else(|| panic!("{token} must parse"));
            assert_eq!(tool.token(), *token);
            assert!(suffixes.insert(tool.suffix()), "duplicate suffix {}", tool.suffix());
            assert!(tool.suffix().starts_with('-'));
        }
        assert!(ImageTool::parse("PNG").is_none());
        assert!(ImageTool::parse("remove_bg").is_none());
        assert_eq!(
            ImageTool::parse(" crop_9_16 "),
            Some(ImageTool::Crop(CropRatio::Portrait9x16))
        );
    }

    #[test]
    fn tools_apply_to_images_only_and_metadata_removal_only_where_metadata_lives() {
        assert!(tools_for_ext("pdf").is_empty());
        assert!(tools_for_ext("mp3").is_empty());
        assert!(tools_for_ext("png").contains(&"CROP_1_1"));
        assert!(tools_for_ext("JPG").contains(&"STRIP_EXIF"));
        assert!(tools_for_ext("heic").contains(&"STRIP_EXIF"));
        for ext in ["gif", "bmp", "ico", "icns", "svg"] {
            assert!(!tools_for_ext(ext).contains(&"STRIP_EXIF"), "{ext}");
            assert!(tools_for_ext(ext).contains(&"PALETTE"), "{ext}");
        }
    }

    #[test]
    fn tools_for_a_selection_are_the_intersection_over_all_files() {
        let none: [&str; 0] = [];
        assert!(tools_for_paths(&none).is_empty());
        let two_photos = ["/a/x.jpg", "/a/y.png"];
        let shared = tools_for_paths(&two_photos);
        assert!(shared.contains(&"CROP_1_1") && shared.contains(&"STRIP_EXIF"));
        // GIF has no metadata to strip, so the shared list loses it but keeps the rest.
        let with_gif = ["/a/x.jpg", "/a/anim.gif"];
        let shared = tools_for_paths(&with_gif);
        assert!(shared.contains(&"ROTATE_RIGHT") && !shared.contains(&"STRIP_EXIF"));
        // One non-image empties the selection's tools entirely.
        assert!(tools_for_paths(&["/a/x.jpg", "/a/doc.pdf"]).is_empty());
    }

    #[test]
    fn redact_is_a_token_with_its_own_suffix_and_the_source_format() {
        assert_eq!(ImageTool::parse("REDACT"), Some(ImageTool::Redact));
        assert_eq!(ImageTool::parse(" redact "), Some(ImageTool::Redact));
        assert_eq!(ImageTool::Redact.token(), "REDACT");
        assert_eq!(ImageTool::Redact.suffix(), "-redacted");
        assert_eq!(ImageTool::Redact.output_format("png"), "PNG");
        assert_eq!(ImageTool::Redact.output_format("jpg"), "JPG");
        assert_eq!(ImageTool::Redact.output_format("gif"), "PNG");
        let position = ALL_TOOL_TOKENS
            .iter()
            .position(|t| *t == "REDACT")
            .expect("REDACT is in the token list");
        assert_eq!(ALL_TOOL_TOKENS[position - 1], "SCALE_1MB");
        assert_eq!(ALL_TOOL_TOKENS[position + 1], "PALETTE");
    }

    #[test]
    fn redact_is_offered_for_every_image_extension_only() {
        for ext in IMAGE_EXTS {
            assert!(tools_for_ext(ext).contains(&"REDACT"), "{ext}");
        }
        assert!(!tools_for_ext("pdf").contains(&"REDACT"));
        assert!(!tools_for_ext("mp3").contains(&"REDACT"));
    }

    #[test]
    fn paint_regions_blacks_the_padded_box_and_leaves_the_rest_untouched() {
        let img = solid(100, 100, [255, 255, 255, 255]);
        // Box 20..40 x 30..40: 20x10 px, padded by max(2, 0.3 * 10) = 3 px per side.
        let out = paint_regions(&img, &[region(0.20, 0.30, 0.20, 0.10)]).to_rgba8();
        for (x, y) in [(17, 27), (20, 30), (39, 39), (42, 42)] {
            assert_eq!(out.get_pixel(x, y).0, [0, 0, 0, 255], "({x},{y}) is inside");
        }
        for (x, y) in [(16, 27), (43, 27), (17, 26), (20, 43)] {
            assert_eq!(out.get_pixel(x, y).0, [255, 255, 255, 255], "({x},{y}) is outside");
        }
    }

    #[test]
    fn paint_regions_clamps_to_the_image_edges() {
        let img = solid(100, 100, [255, 255, 255, 255]);
        let regions = [region(0.0, 0.0, 0.01, 0.01), region(0.95, 0.95, 0.20, 0.20)];
        let out = paint_regions(&img, &regions).to_rgba8();
        for (x, y) in [(0, 0), (2, 2), (89, 89), (99, 99)] {
            assert_eq!(out.get_pixel(x, y).0, [0, 0, 0, 255], "({x},{y}) is clamped into the box");
        }
        for (x, y) in [(20, 20), (88, 99), (99, 88)] {
            assert_eq!(out.get_pixel(x, y).0, [255, 255, 255, 255], "({x},{y}) is outside");
        }
    }

    #[test]
    fn paint_regions_fills_opaque_black_where_the_image_has_alpha() {
        let img = solid(50, 40, [10, 20, 30, 128]);
        let out = paint_regions(&img, &[region(0.1, 0.1, 0.2, 0.2)]).to_rgba8();
        assert_eq!(out.get_pixel(15, 12).0, [0, 0, 0, 255], "covered pixels become opaque black");
        assert_eq!(out.get_pixel(49, 39).0, [10, 20, 30, 128], "the rest keeps its colour and alpha");
    }

    #[test]
    fn redact_refuses_ocr_boxes_for_a_differently_shaped_image() {
        assert!(ensure_same_aspect(1200, 800, 1200, 800).is_ok());
        assert!(ensure_same_aspect(1200, 800, 600, 400).is_ok(), "same ratio, other size");
        assert!(ensure_same_aspect(1200, 800, 1200, 796).is_ok(), "within 1 %");
        assert!(ensure_same_aspect(1200, 800, 800, 1200).is_err(), "swapped axes");
        assert!(ensure_same_aspect(1200, 800, 1200, 760).is_err(), "more than 1 % off");
        assert!(ensure_same_aspect(0, 0, 10, 10).is_err());
        assert!(ensure_same_aspect(1200, 800, 0, 0).is_err());
    }

    #[test]
    fn output_keeps_the_source_format_with_sensible_fallbacks() {
        let rot = ImageTool::RotateRight;
        assert_eq!(rot.output_format("jpg"), "JPG");
        assert_eq!(rot.output_format("JPEG"), "JPG");
        assert_eq!(rot.output_format("png"), "PNG");
        assert_eq!(rot.output_format("webp"), "WEBP");
        assert_eq!(rot.output_format("tif"), "TIFF");
        assert_eq!(rot.output_format("heic"), "JPG");
        assert_eq!(rot.output_format("gif"), "PNG");
        assert_eq!(ImageTool::Palette.output_format("jpg"), "PNG");
    }

    #[test]
    fn centre_crop_has_the_exact_ratio_and_stays_centred() {
        let mut buf = ImageBuffer::from_pixel(400, 300, Rgba([0u8, 0, 0, 255]));
        buf.put_pixel(200, 150, Rgba([255, 0, 0, 255]));
        let img = DynamicImage::ImageRgba8(buf);
        for (ratio, (rw, rh)) in [
            (CropRatio::Portrait9x16, (9u32, 16u32)),
            (CropRatio::Portrait4x5, (4, 5)),
            (CropRatio::Square, (1, 1)),
            (CropRatio::Landscape16x9, (16, 9)),
        ] {
            let c = crop_center(&img, ratio);
            let (w, h) = c.dimensions();
            assert!(w <= 400 && h <= 300);
            assert!(
                (i64::from(w) * i64::from(rh) - i64::from(h) * i64::from(rw)).abs()
                    <= i64::from(rw.max(rh)),
                "{ratio:?}: {w}x{h} is not {rw}:{rh}"
            );
            assert!(w == 400 || h == 300, "the crop must use the full extent of one axis");
            // The red pixel at the image centre must sit within one pixel of the crop's centre.
            let found = (0..w)
                .flat_map(|x| (0..h).map(move |y| (x, y)))
                .find(|&(x, y)| c.get_pixel(x, y).0[0] == 255)
                .expect("centre pixel survives the crop");
            assert!(found.0.abs_diff(w / 2) <= 1 && found.1.abs_diff(h / 2) <= 1, "{ratio:?}: {found:?}");
        }
        assert_eq!(crop_center(&img, CropRatio::Square).dimensions(), (300, 300));
        assert_eq!(crop_center(&img, CropRatio::Portrait9x16).dimensions(), (168, 300));
        assert_eq!(crop_center(&img, CropRatio::Landscape16x9).dimensions(), (400, 225));
        // An already-portrait image keeps its width and trims the height.
        assert_eq!(
            crop_center(&solid(300, 600, [0, 0, 0, 255]), CropRatio::Square).dimensions(),
            (300, 300)
        );
    }

    #[test]
    fn scale_presets_follow_the_documented_geometry_and_never_upscale() {
        assert_eq!(scaled_dimensions(4032, 3024, Scale::ShorterSide(1080)), (1440, 1080));
        assert_eq!(scaled_dimensions(3024, 4032, Scale::ShorterSide(1080)), (1080, 1440));
        assert_eq!(scaled_dimensions(800, 600, Scale::ShorterSide(1080)), (800, 600));
        assert_eq!(scaled_dimensions(1000, 500, Scale::Percent(50)), (500, 250));
        assert_eq!(scaled_dimensions(1000, 500, Scale::Percent(25)), (250, 125));
        assert_eq!(scaled_dimensions(1, 1, Scale::Percent(25)), (1, 1));
    }

    #[test]
    fn dominant_colors_find_the_main_colours_in_order_of_area() {
        let mut buf = ImageBuffer::from_pixel(100, 100, Rgba([200u8, 30, 30, 255]));
        for y in 0..100 {
            for x in 60..100 {
                buf.put_pixel(x, y, Rgba([20, 40, 220, 255]));
            }
        }
        let colors = dominant_colors(&DynamicImage::ImageRgba8(buf), 6);
        assert!(!colors.is_empty() && colors.len() <= 6);
        let first = colors[0].0;
        assert!(first[0] > 150 && first[1] < 80 && first[2] < 80, "red dominates: {first:?}");
        assert!(
            colors.iter().any(|(c, _)| c[2] > 150 && c[0] < 80),
            "blue present: {colors:?}"
        );
        assert!(colors.windows(2).all(|w| w[0].1 >= w[1].1), "sorted by population");
        assert!(
            dominant_colors(&solid(10, 10, [9, 9, 9, 0]), 6).is_empty(),
            "a fully transparent image has no colours"
        );
    }

    #[test]
    fn three_equally_sized_flat_colours_give_exactly_those_three_colours() {
        // Median cut alone averages green with white here; the refinement must keep them apart.
        let mut buf = ImageBuffer::from_pixel(90, 30, Rgba([255u8, 255, 255, 255]));
        for y in 0..30 {
            for x in 0..30 {
                buf.put_pixel(x, y, Rgba([0, 146, 70, 255]));
                buf.put_pixel(x + 60, y, Rgba([206, 43, 55, 255]));
            }
        }
        let colors = dominant_colors(&DynamicImage::ImageRgba8(buf), 6);
        let mut found: Vec<[u8; 3]> = colors.iter().map(|(c, _)| *c).collect();
        found.sort();
        assert_eq!(found, vec![[0, 146, 70], [206, 43, 55], [255, 255, 255]], "{colors:?}");
        assert!(colors.iter().all(|(_, n)| *n == 900), "{colors:?}");
    }

    #[test]
    fn palette_image_has_one_swatch_per_colour_with_a_drawn_label() {
        let colors = vec![([255, 0, 0], 10usize), ([0, 255, 0], 5), ([0, 0, 255], 1)];
        let img = render_palette(&colors);
        assert_eq!(img.dimensions(), (3 * 160, 160 + 44));
        assert_eq!(img.get_pixel(80, 80).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(160 + 80, 80).0, [0, 255, 0]);
        assert_eq!(img.get_pixel(320 + 80, 80).0, [0, 0, 255]);
        let label_has_ink = (0..160u32)
            .flat_map(|x| (160..204u32).map(move |y| (x, y)))
            .any(|(x, y)| img.get_pixel(x, y).0 == [24, 24, 24]);
        assert!(label_has_ink, "the hex label must be drawn under the swatch");
        assert_eq!(hex_label([255, 0, 171]), "#FF00AB");
    }

    fn temp_file(ext: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("flip-tools-{}.{ext}", uuid::Uuid::new_v4().simple()))
    }

    fn jpeg_with_metadata(orientation: u16) -> Vec<u8> {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(32, 16, |x, y| {
            Rgb([(x * 7) as u8, (y * 15) as u8, 90])
        }));
        let path = temp_file("jpg");
        encode_to(&path, &img, "JPG", 90).unwrap();
        let plain = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        // Little-endian TIFF with one IFD0 entry: Orientation.
        let mut tiff = vec![b'I', b'I', 0x2A, 0, 8, 0, 0, 0, 1, 0];
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        app1.extend_from_slice(b"GPS:41.9028N,12.4964E SECRET-CAMERA");
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>SECRET-XMP</x:xmpmeta>".to_vec();
        let mut out = vec![0xFF, 0xD8];
        for body in [&app1, &xmp] {
            out.extend_from_slice(&[0xFF, 0xE1]);
            out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(body);
        }
        // COM segment: the length field counts its own 2 bytes plus the 12-byte body.
        out.extend_from_slice(&[0xFF, 0xFE, 0, 14]);
        out.extend_from_slice(b"SECRET-CMT\0\0");
        out.extend_from_slice(&plain[2..]);
        out
    }

    #[test]
    fn jpeg_strip_removes_exif_xmp_and_comments_but_keeps_the_pixels_byte_identical() {
        let with_meta = jpeg_with_metadata(1);
        assert!(with_meta.windows(13).any(|w| w == b"SECRET-CAMERA"));
        assert_eq!(jpeg_exif_orientation(&with_meta), Some(1));

        let clean = strip_jpeg(&with_meta).unwrap();
        for secret in [&b"SECRET-CAMERA"[..], b"SECRET-XMP", b"SECRET-CMT", b"Exif", b"GPS:"] {
            assert!(!clean.windows(secret.len()).any(|w| w == secret), "leaked {secret:?}");
        }
        assert!(clean.len() < with_meta.len());
        let before = ::image::load_from_memory(&with_meta).unwrap().to_rgb8();
        let after = ::image::load_from_memory(&clean).unwrap().to_rgb8();
        assert_eq!(before.as_raw(), after.as_raw(), "lossless: identical pixels");
        assert_eq!(jpeg_exif_orientation(&clean), None);
        assert!(strip_jpeg(b"not a jpeg").is_err());
    }

    #[test]
    fn jpeg_orientation_is_read_in_both_byte_orders() {
        assert_eq!(jpeg_exif_orientation(&jpeg_with_metadata(6)), Some(6));
        assert_eq!(jpeg_exif_orientation(&jpeg_with_metadata(8)), Some(8));
        // Big-endian TIFF with Orientation = 3.
        let tiff = [
            b'M', b'M', 0, 0x2A, 0, 0, 0, 8, // header, IFD at 8
            0, 1, // one entry
            0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 3, 0, 0, // tag, SHORT, count 1, value 3
            0, 0, 0, 0,
        ];
        assert_eq!(tiff_orientation(&tiff), Some(3));
        assert_eq!(tiff_orientation(b"XX"), None);
    }

    #[test]
    fn png_strip_drops_text_exif_and_time_chunks_and_restores_the_clean_file() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(4, 4, Rgb([10, 20, 30])));
        let path = temp_file("png");
        img.save_with_format(&path, ImageFormat::Png).unwrap();
        let plain = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        let chunk = |kind: &[u8; 4], data: &[u8]| {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(data);
            c.extend_from_slice(&[0, 0, 0, 0]); // CRC is irrelevant to the chunk filter
            c
        };
        // IHDR ends at byte 33 (8 signature + 25 chunk).
        let mut with_meta = plain[..33].to_vec();
        with_meta.extend(chunk(b"tEXt", b"Author\0SECRET-PNG"));
        with_meta.extend(chunk(b"eXIf", b"SECRET-EXIF"));
        with_meta.extend_from_slice(&plain[33..]);

        let clean = strip_png(&with_meta).unwrap();
        assert!(!clean.windows(10).any(|w| w == b"SECRET-PNG"));
        assert!(!clean.windows(11).any(|w| w == b"SECRET-EXIF"));
        assert_eq!(clean, plain, "removing the injected chunks restores the original bytes");
        assert!(::image::load_from_memory(&clean).is_ok());
        assert!(strip_png(b"nope").is_err());
    }

    #[test]
    fn webp_strip_drops_exif_and_xmp_chunks_and_keeps_a_consistent_container() {
        let img = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(8, 8, Rgba([1u8, 2, 3, 255])));
        let path = temp_file("webp");
        img.save_with_format(&path, ImageFormat::WebP).unwrap();
        let plain = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        // Extended container: VP8X (EXIF + XMP flags set) + the image data + EXIF + XMP chunks.
        let mut chunks = Vec::new();
        chunks.extend_from_slice(b"VP8X");
        chunks.extend_from_slice(&10u32.to_le_bytes());
        chunks.extend_from_slice(&[0x0C, 0, 0, 0, 7, 0, 0, 7, 0, 0]);
        chunks.extend_from_slice(&plain[12..]);
        for (kind, body) in [(b"EXIF", &b"SECRET-W-EXIF"[..]), (b"XMP ", b"SECRET-W-XMP!")] {
            chunks.extend_from_slice(kind);
            chunks.extend_from_slice(&(body.len() as u32).to_le_bytes());
            chunks.extend_from_slice(body);
            if body.len() % 2 == 1 {
                chunks.push(0);
            }
        }
        let mut with_meta = b"RIFF".to_vec();
        with_meta.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        with_meta.extend_from_slice(b"WEBP");
        with_meta.extend_from_slice(&chunks);

        let clean = strip_webp(&with_meta).unwrap();
        assert!(!clean.windows(13).any(|w| w == b"SECRET-W-EXIF"));
        assert!(!clean.windows(12).any(|w| w == b"SECRET-W-XMP"));
        assert_eq!(&clean[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes([clean[4], clean[5], clean[6], clean[7]]) as usize,
            clean.len() - 8
        );
        assert_eq!(clean[20] & 0x0C, 0, "VP8X EXIF/XMP flags cleared");
        assert!(strip_webp(b"RIFFxxxxWAVE").is_err());
    }

    #[test]
    fn encode_under_shrinks_until_the_limit_and_fails_cleanly_when_impossible() {
        let noisy = DynamicImage::ImageRgb8(ImageBuffer::from_fn(600, 600, |x, y| {
            Rgb([
                ((x * 31 + y * 17) % 251) as u8,
                ((x * 13 + y * 7) % 241) as u8,
                ((x ^ y) % 256) as u8,
            ])
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let path = temp_file("jpg");
        encode_under(&path, &noisy, "JPG", 95, 60_000, &cancel).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        assert!(size <= 60_000, "size {size}");
        assert!(::image::open(&path).is_ok());
        let _ = fs::remove_file(&path);

        let impossible = temp_file("png");
        assert!(encode_under(&impossible, &noisy, "PNG", 95, 10, &cancel).is_err());
        let _ = fs::remove_file(&impossible);

        let cancelled = Arc::new(AtomicBool::new(true));
        let path = temp_file("jpg");
        assert!(matches!(
            encode_under(&path, &noisy, "JPG", 95, 1, &cancelled),
            Err(EngineError::Cancelled)
        ));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn apply_manual_crop_crops_normalized_rect_and_writes_cropped_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("photo.png");
        let img = solid(200, 100, [40, 120, 200, 255]);
        img.save_with_format(&src, ImageFormat::Png).unwrap();

        let prefs = AppPreferences::default();
        let cancel = AtomicBool::new(false);
        let out = apply_manual_crop(&src, 0.25, 0.10, 0.50, 0.80, &prefs, &cancel).unwrap();

        assert_eq!(out.file_name().and_then(|s| s.to_str()), Some("photo-cropped.png"));
        let decoded = ::image::open(&out).unwrap();
        assert_eq!(decoded.dimensions(), (100, 80));

        assert!(apply_manual_crop(&src, 0.25, 0.10, 0.01, 0.80, &prefs, &cancel).is_err());
        assert!(apply_manual_crop(&src, 0.25, 0.10, 0.50, 0.005, &prefs, &cancel).is_err());
    }
}
