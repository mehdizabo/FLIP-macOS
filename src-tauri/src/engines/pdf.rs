use crate::core::preferences::AppPreferences;
use crate::engines::{bridge, document, DetectedTools, EngineError};
use image::ImageFormat;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub fn convert(
    source: &Path,
    format: &str,
    staging_dir: &Path,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let fmt = format.trim().to_ascii_uppercase();

    if matches!(fmt.as_str(), "PNG" | "JPG" | "HEIC" | "TIFF") {
        unit_progress(35);
        let render_dir = staging_dir.join("pdf-pages");
        let rendered = bridge::convert_pdf_render(
            source,
            &render_dir,
            &fmt,
            prefs.pdf_dpi,
            Some(cancel),
        )?;
        return Ok(rendered);
    }

    if fmt == "WEBP" {
        unit_progress(30);
        let render_dir = staging_dir.join("pdf-pages-png");
        let png_pages = bridge::convert_pdf_render(
            source,
            &render_dir,
            "PNG",
            prefs.pdf_dpi,
            Some(cancel),
        )?;
        let mut webp_pages = Vec::with_capacity(png_pages.len());
        for (idx, png_path) in png_pages.iter().enumerate() {
            let img = image::open(png_path).map_err(|e| {
                EngineError::ConversionFailed(format!("Failed to decode rendered page PNG: {e}"))
            })?;
            let out_webp = staging_dir.join(format!("page-{:03}.webp", idx + 1));
            img.save_with_format(&out_webp, ImageFormat::WebP)
                .map_err(|e| {
                    EngineError::ConversionFailed(format!("Failed to encode page WebP: {e}"))
                })?;
            webp_pages.push(out_webp);
        }
        return Ok(webp_pages);
    }

    unit_progress(35);
    let extracted_txt = staging_dir.join("extracted.txt");
    bridge::convert_pdf_extract_text(source, &extracted_txt, Some(cancel))?;
    let text = fs::read_to_string(&extracted_txt)?;
    if text.trim().is_empty() {
        return Err(EngineError::ConversionFailed(
            "PDF contains no extractable text; Vision OCR found no readable text.".to_string(),
        ));
    }

    if fmt == "TXT" {
        return Ok(vec![extracted_txt]);
    }

    if fmt == "MD" {
        let out_md = staging_dir.join("output.md");
        let title = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Document");
        let md = format!("# {title}\n\n{}\n", text.trim());
        fs::write(&out_md, md)?;
        return Ok(vec![out_md]);
    }

    let html = document::plain_text_to_html(
        source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Document"),
        &text,
    );
    let html_path = staging_dir.join("extracted.html");
    fs::write(&html_path, html)?;

    if fmt == "HTML" {
        return Ok(vec![html_path]);
    }

    if matches!(fmt.as_str(), "EPUB" | "FB2" | "RST") {
        let out_path = staging_dir.join(format!("output.{}", fmt.to_ascii_lowercase()));
        document::convert_via_pandoc(&html_path, "html", &fmt, &out_path, staging_dir, tools, cancel)?;
        return Ok(vec![out_path]);
    }

    if matches!(fmt.as_str(), "DOCX" | "RTF" | "ODT") {
        let out_path = staging_dir.join(format!("output.{}", fmt.to_ascii_lowercase()));
        bridge::convert_doc(&html_path, &fmt, &out_path, &prefs.pdf_page_size, Some(cancel))?;
        return Ok(vec![out_path]);
    }

    Err(EngineError::Unsupported(format!(
        "Unsupported PDF output format: {fmt}"
    )))
}
