//! Parity guard against the original FLIP (Windows, 0.5.1).
//!
//! The tables below are copied from `FLIP-windows/src/Flip/Policy.cs`. For every input extension the
//! original accepts, the port must offer at least the same outputs. The port is allowed to offer more
//! (HEIC, ICO, TOML, XLSX, ZIP, ...); it must never offer less.

use flip_lib::core::policy::{outputs, EngineCapabilities};
use std::path::PathBuf;

const IMAGE_IN: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif"];
const IMAGE_OUT: &[&str] = &["PNG", "JPG", "WEBP", "PDF", "BMP", "TIFF", "GIF"];

const DATA_IN: &[&str] = &["json", "yaml", "yml", "xml", "csv"];
const DATA_OUT: &[&str] = &["JSON", "YAML", "XML", "CSV", "TXT", "HTML", "MD", "PDF"];

const AUDIO_IN: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus"];
const AUDIO_OUT: &[&str] = &["WAV", "MP3", "FLAC", "OGG", "M4A", "AAC"];

const VIDEO_IN: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "wmv", "m4v"];
const VIDEO_OUT: &[&str] = &["MP4", "MP3", "WAV", "MKV", "WEBM", "AVI", "MOV"];

const DOC_IN: &[&str] = &[
    "txt", "md", "markdown", "html", "htm", "docx", "odt", "rtf", "epub", "rst", "fb2",
];
/// Original outputs for documents with Pandoc and LibreOffice available.
const DOC_OUT: &[&str] = &[
    "PDF", "DOCX", "EPUB", "MD", "HTML", "TXT", "ODT", "RTF", "PNG", "JPG", "WEBP", "FB2", "RST",
];

const PDF_IN: &[&str] = &["pdf"];
const PDF_OUT: &[&str] = &[
    "PNG", "JPG", "WEBP", "TXT", "DOCX", "MD", "HTML", "EPUB", "ODT", "RTF", "FB2", "RST",
];

/// The original never lists the source format itself as a target.
fn same_format(ext: &str) -> String {
    match ext {
        "jpeg" => "JPG".to_string(),
        "tif" => "TIFF".to_string(),
        "markdown" => "MD".to_string(),
        "htm" => "HTML".to_string(),
        "yml" => "YAML".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

fn port_outputs(ext: &str, caps: &EngineCapabilities) -> Vec<String> {
    outputs(&[PathBuf::from(format!("/tmp/flip-parity/sample.{ext}"))], caps)
}

fn assert_superset(group: &str, exts: &[&str], original_out: &[&str], caps: &EngineCapabilities) {
    for ext in exts {
        let have = port_outputs(ext, caps);
        let source = same_format(ext);
        let missing: Vec<&&str> = original_out
            .iter()
            .filter(|o| **o != source && !have.iter().any(|h| h == **o))
            .collect();
        assert!(
            missing.is_empty(),
            "{group} .{ext}: the original offers {missing:?} but the port only offers {have:?}"
        );
    }
}

#[test]
fn every_input_extension_of_the_original_offers_at_least_its_outputs_with_all_engines() {
    let all = EngineCapabilities::all();
    assert_superset("image", IMAGE_IN, IMAGE_OUT, &all);
    assert_superset("data", DATA_IN, DATA_OUT, &all);
    assert_superset("audio", AUDIO_IN, AUDIO_OUT, &all);
    assert_superset("video", VIDEO_IN, VIDEO_OUT, &all);
    assert_superset("document", DOC_IN, DOC_OUT, &all);
    assert_superset("pdf", PDF_IN, PDF_OUT, &all);
}

#[test]
fn families_the_original_converts_without_external_engines_stay_native_in_the_port() {
    let native = EngineCapabilities::native_only();
    assert_superset("image", IMAGE_IN, IMAGE_OUT, &native);
    assert_superset("data", DATA_IN, DATA_OUT, &native);
    // Original without Pandoc: PDF -> PNG/JPG/WEBP/TXT and TXT -> PDF/PNG/JPG/WEBP.
    assert_superset("pdf", PDF_IN, &["PNG", "JPG", "WEBP", "TXT"], &native);
    assert_superset("txt", &["txt"], &["PDF", "PNG", "JPG", "WEBP"], &native);
}

#[test]
fn every_original_input_extension_is_recognised_by_the_port() {
    let all = EngineCapabilities::all();
    for ext in IMAGE_IN
        .iter()
        .chain(DATA_IN)
        .chain(AUDIO_IN)
        .chain(VIDEO_IN)
        .chain(DOC_IN)
        .chain(PDF_IN)
    {
        assert!(
            !port_outputs(ext, &all).is_empty(),
            "extension .{ext} is supported by the original but not offered by the port"
        );
    }
}
