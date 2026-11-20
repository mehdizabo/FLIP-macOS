//! End-to-end checks for the image tools ("Strumenti") through the real conversion pipeline.

use flip_lib::core::preferences::AppPreferences;
use flip_lib::engines::convert_files;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgb};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tempfile::tempdir;

fn sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn gradient(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(ImageBuffer::from_fn(w, h, |x, y| {
        Rgb([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, ((x + y) % 256) as u8])
    }))
}

fn noisy(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(ImageBuffer::from_fn(w, h, |x, y| {
        let v = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503));
        Rgb([(v >> 3) as u8, (v >> 11) as u8, (v >> 19) as u8])
    }))
}

fn write_jpeg(path: &Path, img: &DynamicImage, quality: u8) {
    let mut out = std::io::BufWriter::new(fs::File::create(path).unwrap());
    DynamicImage::ImageRgb8(img.to_rgb8())
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, quality))
        .unwrap();
}

fn run(source: &Path, token: &str) -> flip_lib::engines::ConversionResult {
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut results = convert_files(
        std::slice::from_ref(&source.to_path_buf()),
        &[token.to_string()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    results.remove(0)
}

fn only_output(source: &Path, token: &str) -> PathBuf {
    let res = run(source, token);
    assert!(res.error.is_none(), "{token} failed: {:?}", res.error);
    assert_eq!(res.outputs.len(), 1, "{token}: {:?}", res.outputs);
    PathBuf::from(&res.outputs[0])
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn crop_rotate_flip_and_scale_keep_the_format_name_the_result_and_never_touch_the_original() {
    let dir = tempdir().unwrap();
    let jpg = dir.path().join("photo.jpg");
    let png = dir.path().join("logo.png");
    write_jpeg(&jpg, &gradient(400, 300), 92);
    gradient(400, 300).save(&png).unwrap();
    let (jpg_sha, png_sha) = (sha(&jpg), sha(&png));

    let cases: &[(&Path, &str, &str, (u32, u32))] = &[
        (&jpg, "CROP_1_1", "photo-crop-1x1.jpg", (300, 300)),
        (&jpg, "CROP_9_16", "photo-crop-9x16.jpg", (168, 300)),
        (&jpg, "CROP_16_9", "photo-crop-16x9.jpg", (400, 225)),
        (&jpg, "ROTATE_RIGHT", "photo-rotate-right.jpg", (300, 400)),
        (&jpg, "ROTATE_LEFT", "photo-rotate-left.jpg", (300, 400)),
        (&jpg, "ROTATE_180", "photo-rotate-180.jpg", (400, 300)),
        (&jpg, "FLIP_H", "photo-flip-h.jpg", (400, 300)),
        (&jpg, "SCALE_50", "photo-50pct.jpg", (200, 150)),
        (&jpg, "SCALE_25", "photo-25pct.jpg", (100, 75)),
        (&png, "CROP_4_5", "logo-crop-4x5.png", (240, 300)),
        (&png, "ROTATE_RIGHT", "logo-rotate-right.png", (300, 400)),
        (&png, "SCALE_50", "logo-50pct.png", (200, 150)),
    ];
    for (source, token, expected_name, expected_dims) in cases {
        let out = only_output(source, token);
        assert_eq!(file_name(&out), *expected_name, "{token}");
        assert_eq!(image::open(&out).unwrap().dimensions(), *expected_dims, "{token}");
    }
    assert_eq!(sha(&jpg), jpg_sha, "the original JPG is never modified");
    assert_eq!(sha(&png), png_sha, "the original PNG is never modified");
}

#[test]
fn rotating_right_moves_the_top_left_pixel_to_the_top_right_and_left_does_the_opposite() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("marker.png");
    let mut buf = ImageBuffer::from_pixel(40, 20, Rgb([0u8, 0, 0]));
    buf.put_pixel(0, 0, Rgb([255, 0, 0]));
    DynamicImage::ImageRgb8(buf).save(&src).unwrap();

    let right = image::open(only_output(&src, "ROTATE_RIGHT")).unwrap().to_rgb8();
    assert_eq!(right.dimensions(), (20, 40));
    assert_eq!(right.get_pixel(19, 0).0, [255, 0, 0], "top-left goes to top-right");
    let left = image::open(only_output(&src, "ROTATE_LEFT")).unwrap().to_rgb8();
    assert_eq!(left.get_pixel(0, 39).0, [255, 0, 0], "top-left goes to bottom-left");
    let flipped = image::open(only_output(&src, "FLIP_H")).unwrap().to_rgb8();
    assert_eq!(flipped.get_pixel(39, 0).0, [255, 0, 0], "mirrored left-right");
    let upside = image::open(only_output(&src, "FLIP_V")).unwrap().to_rgb8();
    assert_eq!(upside.get_pixel(0, 19).0, [255, 0, 0], "mirrored top-bottom");
}

#[test]
fn scale_presets_hit_their_targets_and_the_1080p_preset_never_upscales() {
    let dir = tempdir().unwrap();
    let big = dir.path().join("big.jpg");
    write_jpeg(&big, &gradient(2160, 1620), 85);
    let out = only_output(&big, "SCALE_1080P");
    assert_eq!(file_name(&out), "big-1080p.jpg");
    assert_eq!(image::open(&out).unwrap().dimensions(), (1440, 1080));

    let small = dir.path().join("small.jpg");
    write_jpeg(&small, &gradient(640, 480), 85);
    let out = only_output(&small, "SCALE_1080P");
    assert_eq!(image::open(&out).unwrap().dimensions(), (640, 480), "already smaller: kept as is");

    let heavy = dir.path().join("heavy.jpg");
    write_jpeg(&heavy, &noisy(1600, 1600), 100);
    assert!(fs::metadata(&heavy).unwrap().len() > 1_000_000, "fixture must start above 1 MB");
    let out = only_output(&heavy, "SCALE_1MB");
    assert_eq!(file_name(&out), "heavy-under-1mb.jpg");
    assert!(fs::metadata(&out).unwrap().len() <= 1_000_000);
    assert!(image::open(&out).is_ok());
}

#[test]
fn palette_writes_a_png_swatch_strip_with_the_dominant_colours() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("flag.png");
    let mut buf = ImageBuffer::from_pixel(90, 30, Rgb([255u8, 255, 255]));
    for y in 0..30 {
        for x in 0..30 {
            buf.put_pixel(x, y, Rgb([0, 146, 70]));
            buf.put_pixel(x + 60, y, Rgb([206, 43, 55]));
        }
    }
    DynamicImage::ImageRgb8(buf).save(&src).unwrap();

    let out = only_output(&src, "PALETTE");
    assert_eq!(file_name(&out), "flag-palette.png");
    let strip = image::open(&out).unwrap().to_rgb8();
    assert_eq!(strip.height(), 160 + 44);
    assert_eq!(strip.width() % 160, 0);
    let swatches = strip.width() / 160;
    assert!((3..=6).contains(&swatches), "three flat colours -> {swatches} swatches");
    let centres: Vec<[u8; 3]> = (0..swatches).map(|i| strip.get_pixel(i * 160 + 80, 80).0).collect();
    for wanted in [[0u8, 146, 70], [206, 43, 55], [255, 255, 255]] {
        assert!(
            centres.iter().any(|c| c.iter().zip(wanted).all(|(a, b)| a.abs_diff(b) < 8)),
            "{wanted:?} missing from {centres:?}"
        );
    }
}

fn inject_app1_secrets(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = vec![b'I', b'I', 0x2A, 0, 8, 0, 0, 0, 1, 0];
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    app1.extend_from_slice(b"GPSLatitude=41.9028 Make=SECRET-CAMERA");
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn strip_exif_removes_camera_and_gps_data_losslessly_and_leaves_the_original_alone() {
    let dir = tempdir().unwrap();
    let plain = dir.path().join("plain.jpg");
    write_jpeg(&plain, &gradient(160, 120), 90);
    let src = dir.path().join("holiday.jpg");
    fs::write(&src, inject_app1_secrets(&fs::read(&plain).unwrap(), 1)).unwrap();
    let before = sha(&src);
    assert!(fs::read(&src).unwrap().windows(13).any(|w| w == b"SECRET-CAMERA"));

    let out = only_output(&src, "STRIP_EXIF");
    assert_eq!(file_name(&out), "holiday-clean.jpg");
    let bytes = fs::read(&out).unwrap();
    for secret in [&b"SECRET-CAMERA"[..], b"GPSLatitude", b"Exif"] {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret), "leaked {secret:?}");
    }
    let a = image::open(&src).unwrap().to_rgb8();
    let b = image::open(&out).unwrap().to_rgb8();
    assert_eq!(a.as_raw(), b.as_raw(), "metadata removal must not touch the pixels");
    assert_eq!(sha(&src), before, "the original keeps its metadata");
}

#[test]
fn strip_exif_bakes_in_a_non_default_orientation_instead_of_leaving_the_photo_sideways() {
    let dir = tempdir().unwrap();
    let plain = dir.path().join("plain.jpg");
    // Stored 80x40 with EXIF orientation 6 (rotate 90° CW to display): shown upright as 40x80.
    write_jpeg(&plain, &gradient(80, 40), 95);
    let src = dir.path().join("portrait.jpg");
    fs::write(&src, inject_app1_secrets(&fs::read(&plain).unwrap(), 6)).unwrap();

    let out = only_output(&src, "STRIP_EXIF");
    let bytes = fs::read(&out).unwrap();
    assert!(!bytes.windows(13).any(|w| w == b"SECRET-CAMERA"));
    assert_eq!(
        image::open(&out).unwrap().dimensions(),
        (40, 80),
        "the orientation is applied to the pixels because the tag is gone"
    );
}

#[test]
fn strip_exif_cleans_png_text_chunks_without_re_encoding() {
    let dir = tempdir().unwrap();
    let base = dir.path().join("base.png");
    gradient(16, 16).save(&base).unwrap();
    let raw = fs::read(&base).unwrap();
    // A well-formed tEXt chunk: length, type, data, CRC-32 over type + data.
    let data = b"Comment\0SECRET-PNG-TXT";
    let mut crc = 0xFFFF_FFFFu32;
    for byte in b"tEXt".iter().chain(data.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    let mut text = (data.len() as u32).to_be_bytes().to_vec();
    text.extend_from_slice(b"tEXt");
    text.extend_from_slice(data);
    text.extend_from_slice(&(!crc).to_be_bytes());
    let mut with_text = raw[..33].to_vec();
    with_text.extend_from_slice(&text);
    with_text.extend_from_slice(&raw[33..]);
    let src = dir.path().join("shot.png");
    fs::write(&src, &with_text).unwrap();

    let out = only_output(&src, "STRIP_EXIF");
    assert_eq!(file_name(&out), "shot-clean.png");
    assert!(!fs::read(&out).unwrap().windows(14).any(|w| w == b"SECRET-PNG-TX"));
    assert_eq!(fs::read(&out).unwrap(), raw);
}

#[test]
fn tools_never_overwrite_an_existing_result_and_reject_files_they_do_not_apply_to() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("photo.jpg");
    write_jpeg(&src, &gradient(100, 100), 90);

    let first = only_output(&src, "ROTATE_RIGHT");
    let second = only_output(&src, "ROTATE_RIGHT");
    assert_eq!(file_name(&first), "photo-rotate-right.jpg");
    assert_eq!(file_name(&second), "photo-rotate-right 2.jpg");
    assert_ne!(first, second);

    let pdf = dir.path().join("doc.pdf");
    fs::write(&pdf, b"%PDF-1.4\n%%EOF").unwrap();
    let rejected = run(&pdf, "CROP_1_1");
    assert!(rejected.outputs.is_empty());
    assert!(
        rejected.error.as_deref().is_some_and(|e| e.contains("unavailable") || e.contains("not")),
        "{:?}",
        rejected.error
    );

    let gif = dir.path().join("anim.gif");
    gradient(20, 20).save(&gif).unwrap();
    let no_meta = run(&gif, "STRIP_EXIF");
    assert!(no_meta.error.is_some(), "GIF carries no metadata: the tool does not apply");
}

#[test]
fn inspection_offers_tools_only_when_every_selected_file_is_an_image() {
    use flip_lib::host::inspect_paths_with_prefs;
    let dir = tempdir().unwrap();
    let jpg = dir.path().join("a.jpg");
    let png = dir.path().join("b.png");
    let pdf = dir.path().join("c.pdf");
    write_jpeg(&jpg, &gradient(20, 20), 90);
    gradient(20, 20).save(&png).unwrap();
    fs::write(&pdf, b"%PDF-1.4\n%%EOF").unwrap();
    let prefs = AppPreferences::default();
    let path = |p: &Path| p.to_string_lossy().into_owned();

    let one = inspect_paths_with_prefs(&[path(&jpg)], &prefs);
    assert!(one.tool_actions.iter().any(|t| t == "CROP_1_1"), "{:?}", one.tool_actions);
    assert!(one.tool_actions.iter().any(|t| t == "STRIP_EXIF"));
    assert!(!one.compatible_formats.iter().any(|f| f.starts_with("CROP_")), "tools never leak into the format list");

    let two = inspect_paths_with_prefs(&[path(&jpg), path(&png)], &prefs);
    assert_eq!(two.tool_actions, one.tool_actions);

    let mixed = inspect_paths_with_prefs(&[path(&jpg), path(&pdf)], &prefs);
    assert!(mixed.tool_actions.is_empty());
    assert!(inspect_paths_with_prefs(&[path(&pdf)], &prefs).tool_actions.is_empty());
    assert!(inspect_paths_with_prefs(&[], &prefs).tool_actions.is_empty());
}

fn redact_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// Boxes of the fixture's sensitive values, normalised to the upright image.
fn expected_redact_boxes() -> Vec<[f64; 4]> {
    let raw = fs::read_to_string(redact_fixture("redact-sample.expected.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let b = entry["box"].as_array().unwrap();
            assert_eq!(b.len(), 4, "{entry}");
            [
                b[0].as_f64().unwrap(),
                b[1].as_f64().unwrap(),
                b[2].as_f64().unwrap(),
                b[3].as_f64().unwrap(),
            ]
        })
        .collect()
}

fn pixel_rect(w: u32, h: u32, [x, y, bw, bh]: [f64; 4]) -> (u32, u32, u32, u32) {
    let wf = f64::from(w);
    let hf = f64::from(h);
    (
        (x * wf).floor().max(0.0) as u32,
        (y * hf).floor().max(0.0) as u32,
        ((x + bw) * wf).ceil().clamp(0.0, wf) as u32,
        ((y + bh) * hf).ceil().clamp(0.0, hf) as u32,
    )
}

fn mean_luminance(img: &image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let [r, g, b] = img.get_pixel(x, y).0;
            sum += 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
            count += 1;
        }
    }
    assert!(count > 0, "box ({x0},{y0})-({x1},{y1}) is empty");
    sum / count as f64
}

fn dark_pixels(img: &image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> u64 {
    let mut dark = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let [r, g, b] = img.get_pixel(x, y).0;
            let luma = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
            if luma < 100.0 {
                dark += 1;
            }
        }
    }
    dark
}

fn dir_entries(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect()
}

#[test]
fn redact_covers_the_sensitive_values_in_a_png_and_keeps_the_uncovered_text() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("redact-sample.png");
    fs::copy(redact_fixture("redact-sample.png"), &src).unwrap();
    let before = sha(&src);

    let out = only_output(&src, "REDACT");
    assert_eq!(file_name(&out), "redact-sample-redacted.png");
    assert_eq!(out.parent(), Some(dir.path()), "the result sits next to the source");
    assert_eq!(sha(&src), before, "the original is never modified");

    let after = image::open(&out).unwrap().to_rgb8();
    assert_eq!(after.dimensions(), (1200, 800));
    for expected in expected_redact_boxes() {
        let (x0, y0, x1, y1) = pixel_rect(after.width(), after.height(), expected);
        let mean = mean_luminance(&after, x0, y0, x1, y1);
        assert!(mean < 10.0, "{expected:?} is not covered: mean luminance {mean:.1}");
    }

    // The "Invoice 12/03/2025 total …" line below the last covered box keeps its dark text.
    let band_top = (0.74 * f64::from(after.height())) as u32;
    let band_bottom = (0.95 * f64::from(after.height())) as u32;
    let invoice_ink = dark_pixels(&after, 0, band_top, after.width(), band_bottom);
    assert!(invoice_ink > 50, "the uncovered Invoice line must still show its text ({invoice_ink} dark px)");
}

#[test]
fn redact_applies_upright_boxes_to_an_exif_rotated_jpeg_and_writes_no_exif() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("redact-sample-rot6.jpg");
    fs::copy(redact_fixture("redact-sample-rot6.jpg"), &src).unwrap();
    let before = sha(&src);

    let out = only_output(&src, "REDACT");
    assert_eq!(file_name(&out), "redact-sample-rot6-redacted.jpg");
    assert_eq!(sha(&src), before, "the original is never modified");

    let bytes = fs::read(&out).unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"Exif"), "the result carries no EXIF");
    assert_eq!(flip_lib::engines::image_tools::jpeg_exif_orientation(&bytes), None);

    let after = image::open(&out).unwrap().to_rgb8();
    assert_eq!(after.dimensions(), (1200, 800), "upright: the stored rotation is baked in");
    for expected in expected_redact_boxes() {
        let (x0, y0, x1, y1) = pixel_rect(after.width(), after.height(), expected);
        let mean = mean_luminance(&after, x0, y0, x1, y1);
        assert!(mean < 10.0, "{expected:?} is not covered in upright space: mean {mean:.1}");
    }
}

#[test]
fn redact_refuses_an_image_without_sensitive_text_and_writes_nothing() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("plain.png");
    DynamicImage::ImageRgb8(ImageBuffer::from_pixel(320, 240, Rgb([200u8, 200, 200])))
        .save(&src)
        .unwrap();
    let before = dir_entries(dir.path());

    let res = run(&src, "REDACT");
    assert!(res.error.is_some(), "no sensitive data must be an error");
    assert!(res.outputs.is_empty());
    assert_eq!(dir_entries(dir.path()), before, "no file is written next to the source");
}
