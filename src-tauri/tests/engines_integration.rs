use flip_lib::core::policy::{
    EngineCapabilities, MAX_CONTAINER_ENTRIES, MAX_CONTAINER_EXPANDED_BYTES,
};
use flip_lib::core::preferences::AppPreferences;
use flip_lib::engines::create_new::{
    built_in_kinds, create_new_item, ensure_templates_dir_initialized, scan_user_templates,
};
use flip_lib::engines::{bridge, convert_files, detect_tools, media, resolve_target_dir, EngineError};
use image::{GenericImageView, ImageBuffer, ImageFormat, Rgba};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("read file for sha256");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn make_transparent_test_png(path: &Path) {
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(80, 60);
    for y in 0..60 {
        for x in 0..80 {
            if (20..60).contains(&x) && (10..40).contains(&y) {
                img.put_pixel(x, y, Rgba([220, 25, 25, 255]));
            } else {
                img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
    img.save_with_format(path, ImageFormat::Png)
        .expect("save test png");
}

#[test]
fn image_matrix_exif_alpha_collisions_and_combine_pdf() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    let png = dir.path().join("immagine è.png");
    let jpg = dir.path().join("sorgente.jpg");
    let webp = dir.path().join("sorgente.webp");
    make_transparent_test_png(&png);
    make_transparent_test_png(&webp);
    image::open(&png)
        .unwrap()
        .to_rgb8()
        .save_with_format(&jpg, ImageFormat::Jpeg)
        .unwrap();

    // Existing file with direct stem name must NEVER be overwritten
    let collision = dir.path().join("immagine è.jpg");
    fs::write(&collision, b"DO NOT OVERWRITE").unwrap();
    let collision_hash = sha256_file(&collision);

    let mut hashes: HashMap<PathBuf, String> = HashMap::new();
    hashes.insert(png.clone(), sha256_file(&png));
    hashes.insert(jpg.clone(), sha256_file(&jpg));
    hashes.insert(webp.clone(), sha256_file(&webp));

    let formats: Vec<String> = ["JPG", "PDF", "WEBP", "HEIC", "BMP", "TIFF", "GIF"]
        .into_iter()
        .map(String::from)
        .collect();

    let res = convert_files(std::slice::from_ref(&png), &formats, &prefs, &cancel, None).unwrap();
    for r in &res {
        assert!(r.error.is_none(), "error for {}: {:?}", r.format, r.error);
        assert!(!r.outputs.is_empty());
    }

    // Verify collision preserved & output got ` 2.jpg`
    assert_eq!(sha256_file(&collision), collision_hash);
    let jpg_out = PathBuf::from(&res.iter().find(|r| r.format == "JPG").unwrap().outputs[0]);
    assert_eq!(jpg_out.file_name().unwrap().to_str().unwrap(), "immagine è 2.jpg");
    let jpg_bytes = fs::read(&jpg_out).unwrap();
    assert_eq!(&jpg_bytes[0..2], &[0xFF, 0xD8], "valid JPEG signature");

    let decoded = image::open(&jpg_out).unwrap();
    assert_eq!(decoded.dimensions(), (80, 60));
    let corner = decoded.get_pixel(0, 0);
    assert!(
        corner[0] > 245 && corner[1] > 245 && corner[2] > 245,
        "transparent alpha must flatten onto white, got {:?}",
        corner
    );

    // WebP -> PNG, JPG, HEIC
    let webp_res = convert_files(
        std::slice::from_ref(&webp),
        &["PNG".into(), "JPG".into(), "HEIC".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(webp_res.iter().all(|r| r.error.is_none()));

    // EXIF orientation 6 (80x60 rotated -> 60x80 upright)
    let jpg_raw = fs::read(&jpg).unwrap();
    let exif_app1: [u8; 36] = [
        255, 225, 0, 34, 69, 120, 105, 102, 0, 0, 73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0,
        1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let rotated = dir.path().join("orientation-6.jpg");
    let mut rot_bytes = Vec::with_capacity(jpg_raw.len() + exif_app1.len());
    rot_bytes.extend_from_slice(&jpg_raw[..2]);
    rot_bytes.extend_from_slice(&exif_app1);
    rot_bytes.extend_from_slice(&jpg_raw[2..]);
    fs::write(&rotated, rot_bytes).unwrap();
    hashes.insert(rotated.clone(), sha256_file(&rotated));

    let rot_res = convert_files(std::slice::from_ref(&rotated), &["PNG".into()], &prefs, &cancel, None).unwrap();
    assert!(rot_res[0].error.is_none());
    let rot_png = PathBuf::from(&rot_res[0].outputs[0]);
    let oriented = image::open(&rot_png).unwrap();
    assert_eq!(oriented.dimensions(), (60, 80), "EXIF orientation 6 must swap dimensions");

    // Combine multiple images into one multi-page PDF
    let combined = convert_files(
        &[png.clone(), jpg.clone()],
        &["COMBINE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(combined[0].error.is_none());
    let comb_pdf = PathBuf::from(&combined[0].outputs[0]);
    assert!(fs::read(&comb_pdf).unwrap().starts_with(b"%PDF-"));

    // Corrupt image rejected cleanly
    let corrupt = dir.path().join("corrupt.png");
    fs::write(&corrupt, b"not pixel bytes").unwrap();
    let bad = convert_files(&[corrupt], &["JPG".into()], &prefs, &cancel, None).unwrap();
    assert!(bad[0].error.is_some());
    assert!(bad[0].outputs.is_empty());

    // Original hashes unchanged and zero leftover .flip-job-* directories
    for (p, h) in hashes {
        assert_eq!(sha256_file(&p), h, "source mutated: {}", p.display());
    }
    for entry in fs::read_dir(dir.path()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".flip-job-"), "residual job dir: {name}");
    }
}

#[test]
fn structured_data_full_matrix_json_yaml_toml_xml_csv_html_md_pdf() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    let json_path = dir.path().join("users.json");
    let json_content = r#"[
      {"name":"Marco","city":"Milano","active":true,"meta":{"role":"admin"}},
      {"name":"Giulia","city":"Roma","active":false,"meta":{"role":"editor"}}
    ]"#;
    fs::write(&json_path, json_content).unwrap();
    let json_hash = sha256_file(&json_path);

    let targets: Vec<String> = ["YAML", "TOML", "XML", "CSV", "TXT", "MD", "HTML", "PDF"]
        .into_iter()
        .map(String::from)
        .collect();
    let res = convert_files(std::slice::from_ref(&json_path), &targets, &prefs, &cancel, None).unwrap();
    for r in &res {
        assert!(r.error.is_none(), "data {} failed: {:?}", r.format, r.error);
        assert_eq!(r.outputs.len(), 1);
    }

    // CSV must flatten nested `meta.role`
    let csv_out = PathBuf::from(&res.iter().find(|r| r.format == "CSV").unwrap().outputs[0]);
    let csv_text = fs::read_to_string(&csv_out).unwrap();
    assert!(csv_text.contains("meta.role"), "expected dotted key flattening: {csv_text}");
    assert!(csv_text.contains("Milano") && csv_text.contains("Giulia"));

    // CSV -> JSON roundtrip
    let csv_to_json = convert_files(std::slice::from_ref(&csv_out), &["JSON".into()], &prefs, &cancel, None).unwrap();
    assert!(csv_to_json[0].error.is_none());
    let round_json = fs::read_to_string(&csv_to_json[0].outputs[0]).unwrap();
    assert!(round_json.contains("Milano") && round_json.contains("Giulia"));

    // YAML -> TOML -> JSON roundtrip
    let yaml_out = PathBuf::from(&res.iter().find(|r| r.format == "YAML").unwrap().outputs[0]);
    let yaml_to_toml = convert_files(&[yaml_out], &["TOML".into()], &prefs, &cancel, None).unwrap();
    assert!(yaml_to_toml[0].error.is_none(), "yaml->toml: {:?}", yaml_to_toml[0].error);
    let toml_out = PathBuf::from(&yaml_to_toml[0].outputs[0]);
    let toml_to_json = convert_files(&[toml_out], &["JSON".into()], &prefs, &cancel, None).unwrap();
    assert!(toml_to_json[0].error.is_none());
    let from_toml = fs::read_to_string(&toml_to_json[0].outputs[0]).unwrap();
    assert!(from_toml.contains("Marco") && from_toml.contains("Roma"));

    // XML -> JSON
    let xml_out = PathBuf::from(&res.iter().find(|r| r.format == "XML").unwrap().outputs[0]);
    let xml_to_json = convert_files(&[xml_out], &["JSON".into()], &prefs, &cancel, None).unwrap();
    assert!(xml_to_json[0].error.is_none());
    assert!(fs::read_to_string(&xml_to_json[0].outputs[0])
        .unwrap()
        .contains("Marco"));

    assert_eq!(sha256_file(&json_path), json_hash);
}

#[test]
fn documents_and_pdf_multipage_ocr_merge_and_container_bomb() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let tools = detect_tools(Some(&prefs));

    // 182-line UTF-8 Italian text document
    let txt = dir.path().join("testo italiano.txt");
    let mut txt_body = String::from("Flip verifica testo: caffè, città, perché.\nOriginale da conservare.\n");
    for i in 1..=180 {
        txt_body.push_str(&format!("Riga {i} del documento locale.\n"));
    }
    fs::write(&txt, &txt_body).unwrap();
    let txt_hash = sha256_file(&txt);

    let mut doc_targets: Vec<String> = ["PDF", "DOCX", "MD", "HTML", "RTF", "ODT", "EPUB", "PNG", "JPG", "WEBP"]
        .into_iter()
        .map(String::from)
        .collect();
    if tools.pandoc.is_some() {
        doc_targets.extend(["FB2", "RST"].into_iter().map(String::from));
    }

    let doc_res = convert_files(std::slice::from_ref(&txt), &doc_targets, &prefs, &cancel, None).unwrap();
    for r in &doc_res {
        assert!(r.error.is_none(), "doc {} failed: {:?}", r.format, r.error);
        assert!(!r.outputs.is_empty());
    }

    // Multi-page PDF -> PNG / WEBP / TXT / DOCX
    let pdf_path = PathBuf::from(&doc_res.iter().find(|r| r.format == "PDF").unwrap().outputs[0]);
    let png_pages = &doc_res.iter().find(|r| r.format == "PNG").unwrap().outputs;
    assert!(png_pages.len() >= 3, "expected >=3 A4 pages, got {}", png_pages.len());
    assert!(png_pages[0].contains("-p001.png"));

    let pdf_res = convert_files(
        std::slice::from_ref(&pdf_path),
        &["PNG".into(), "JPG".into(), "WEBP".into(), "HEIC".into(), "TXT".into(), "DOCX".into(), "MD".into(), "HTML".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    for r in &pdf_res {
        assert!(r.error.is_none(), "pdf -> {} failed: {:?}", r.format, r.error);
    }
    let extracted = fs::read_to_string(&pdf_res.iter().find(|r| r.format == "TXT").unwrap().outputs[0]).unwrap();
    assert!(extracted.contains("caffè") && extracted.contains("Riga 180"));

    // DOCX / RTF / ODT roundtrips -> TXT preserve UTF-8 text
    for fmt in ["DOCX", "RTF", "ODT", "HTML"] {
        let p = PathBuf::from(&doc_res.iter().find(|r| r.format == fmt).unwrap().outputs[0]);
        let rt = convert_files(&[p], &["TXT".into()], &prefs, &cancel, None).unwrap();
        assert!(rt[0].error.is_none(), "{fmt} -> TXT failed: {:?}", rt[0].error);
        let rt_text = fs::read_to_string(&rt[0].outputs[0]).unwrap();
        assert!(rt_text.contains("caffè"), "{fmt} roundtrip lost caffè");
    }

    // Markdown -> DOCX -> MD roundtrip
    let md_path = dir.path().join("sample.md");
    fs::write(&md_path, "# Title Flip\n\nBold **caffè** text.\n\n- One\n- Two\n").unwrap();
    let md_to_docx = convert_files(&[md_path], &["DOCX".into(), "PDF".into(), "HTML".into()], &prefs, &cancel, None).unwrap();
    assert!(md_to_docx.iter().all(|r| r.error.is_none()));
    let docx_out = PathBuf::from(&md_to_docx.iter().find(|r| r.format == "DOCX").unwrap().outputs[0]);
    let docx_to_md = convert_files(&[docx_out], &["MD".into()], &prefs, &cancel, None).unwrap();
    assert!(docx_to_md[0].error.is_none());
    let md_back = fs::read_to_string(&docx_to_md[0].outputs[0]).unwrap();
    assert!(md_back.contains("Title Flip") && md_back.contains("caffè"));

    // Scanned PDF with real text (flip-cover.png -> PDF -> TXT via Vision OCR)
    let cover_png = repo_root().join("docs/assets/flip-cover.png");
    let scanned_text_pdf = dir.path().join("scanned-text.pdf");
    bridge::convert_pdf_from_images(&scanned_text_pdf, &[cover_png], None).unwrap();
    let ocr_pdf_res = convert_files(std::slice::from_ref(&scanned_text_pdf), &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(ocr_pdf_res[0].error.is_none(), "OCR PDF failed: {:?}", ocr_pdf_res[0].error);
    let ocr_text = fs::read_to_string(&ocr_pdf_res[0].outputs[0]).unwrap();
    assert!(ocr_text.contains("FLIP"), "OCR PDF missed FLIP: {ocr_text}");

    // Blank shape PDF (no text anywhere) -> TXT rejects with OCR error rather than empty file
    let blank_img = dir.path().join("shape.png");
    make_transparent_test_png(&blank_img);
    let blank_pdf = dir.path().join("shape.pdf");
    bridge::convert_pdf_from_images(&blank_pdf, &[blank_img], None).unwrap();
    let blank_res = convert_files(std::slice::from_ref(&blank_pdf), &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(blank_res[0].error.as_deref().unwrap_or("").contains("OCR"));

    // Merge PDFs
    let merged = convert_files(&[pdf_path, scanned_text_pdf], &["MERGE_PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(merged[0].error.is_none());

    // Container bomb (> 256 MiB) rejected before conversion
    let bomb = dir.path().join("bomb.docx");
    {
        let file = File::create(&bomb).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("word/document.xml", opts).unwrap();
        let zeros = vec![0u8; 1024 * 1024];
        for _ in 0..257 {
            zip.write_all(&zeros).unwrap();
        }
        zip.finish().unwrap();
    }
    let bomb_res = convert_files(&[bomb], &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(bomb_res[0].error.as_deref().unwrap_or("").contains("256 MiB"));
    assert!(bomb_res[0].outputs.is_empty());

    assert_eq!(sha256_file(&txt), txt_hash);
}

#[test]
fn media_native_and_ffmpeg_conversions_and_cancellation() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let tools = detect_tools(Some(&prefs));

    let tick_wav = dir.path().join("tone.wav");
    fs::copy(repo_root().join("src-tauri/resources/flip-tick.wav"), &tick_wav).unwrap();

    // Native macOS audio conversion (simulating ffmpeg = None)
    let mut native_tools = tools.clone();
    native_tools.ffmpeg = None;
    let native_m4a = dir.path().join("native.m4a");
    media::convert(&tick_wav, "M4A", &native_m4a, &native_tools, &cancel, &|_| {}).unwrap();
    assert!(fs::metadata(&native_m4a).unwrap().len() > 0);
    let native_aiff = dir.path().join("native.aiff");
    media::convert(&tick_wav, "AIFF", &native_aiff, &native_tools, &cancel, &|_| {}).unwrap();
    assert!(fs::metadata(&native_aiff).unwrap().len() > 0);

    // Full FFmpeg pipeline when ffmpeg is installed
    if let Some(ref ffmpeg) = tools.ffmpeg {
        let res = convert_files(
            std::slice::from_ref(&tick_wav),
            &[
                "MP3".into(),
                "FLAC".into(),
                "OGG".into(),
                "OPUS".into(),
                "M4A".into(),
                "AAC".into(),
                "ALAC".into(),
            ],
            &prefs,
            &cancel,
            None,
        )
        .unwrap();
        for r in &res {
            assert!(r.error.is_none(), "audio {} failed: {:?}", r.format, r.error);
        }

        let mp3_out = PathBuf::from(&res.iter().find(|r| r.format == "MP3").unwrap().outputs[0]);
        let mp3_to_wav = convert_files(&[mp3_out], &["WAV".into()], &prefs, &cancel, None).unwrap();
        assert!(mp3_to_wav[0].error.is_none());
        let wav_hdr = fs::read(&mp3_to_wav[0].outputs[0]).unwrap();
        assert_eq!(&wav_hdr[0..4], b"RIFF");
        assert_eq!(&wav_hdr[8..12], b"WAVE");

        // Synthetic 1s MP4 -> MOV, M4A, GIF
        let mp4 = dir.path().join("clip.mp4");
        let gen = Command::new(ffmpeg)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x240:d=1",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=1",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-shortest",
                mp4.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(gen.status.success(), "ffmpeg fixture gen failed");

        // Native AVFoundation video conversions (simulating ffmpeg = None)
        let native_mov = dir.path().join("native.mov");
        media::convert(&mp4, "MOV", &native_mov, &native_tools, &cancel, &|_| {}).unwrap();
        assert!(fs::metadata(&native_mov).unwrap().len() > 0);
        let native_vm4a = dir.path().join("native_v.m4a");
        media::convert(&mp4, "M4A", &native_vm4a, &native_tools, &cancel, &|_| {}).unwrap();
        assert!(fs::metadata(&native_vm4a).unwrap().len() > 0);

        let vres = convert_files(
            std::slice::from_ref(&mp4),
            &["MOV".into(), "M4V".into(), "M4A".into(), "WAV".into(), "MKV".into(), "WEBM".into(), "AVI".into(), "GIF".into()],
            &prefs,
            &cancel,
            None,
        )
        .unwrap();
        for r in &vres {
            assert!(r.error.is_none(), "video {} failed: {:?}", r.format, r.error);
        }
        let gif_bytes = fs::read(&vres.iter().find(|r| r.format == "GIF").unwrap().outputs[0]).unwrap();
        assert!(gif_bytes.starts_with(b"GIF8"), "valid GIF signature");
    }

    // Cancellation respected immediately before starting
    let cancelled = Arc::new(AtomicBool::new(true));
    let cres = convert_files(&[tick_wav], &["M4A".into()], &prefs, &cancelled, None);
    assert!(matches!(cres, Err(EngineError::Cancelled)));
}

#[test]
fn create_new_builtin_office_ooxml_user_templates_and_readonly_guard() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();

    let kinds = built_in_kinds(false);
    assert_eq!(kinds.iter().filter(|k| k.page == 1).count(), 8);
    assert!(
        !kinds.iter().find(|k| k.id == "clipboard").unwrap().available,
        "clipboard must be disabled when empty"
    );

    // Create Folder + collisions
    let f1 = create_new_item("folder", Some(dir.path()), &prefs, false).unwrap();
    assert_eq!(
        Path::new(&f1.path).file_name().unwrap().to_str().unwrap(),
        "untitled folder"
    );
    assert!(Path::new(&f1.path).is_dir());

    let f2 = create_new_item("folder", Some(dir.path()), &prefs, false).unwrap();
    assert_eq!(
        Path::new(&f2.path).file_name().unwrap().to_str().unwrap(),
        "untitled folder 2"
    );

    // Create plain text + collision
    let t1 = create_new_item("txt", Some(dir.path()), &prefs, false).unwrap();
    assert_eq!(Path::new(&t1.path).file_name().unwrap().to_str().unwrap(), "Untitled.txt");
    let t2 = create_new_item("txt", Some(dir.path()), &prefs, false).unwrap();
    assert_eq!(Path::new(&t2.path).file_name().unwrap().to_str().unwrap(), "Untitled 2.txt");

    // Create Markdown, RTF, HTML, JSON, CSV, Shell Script (+x), Python, Swift
    for (id, expected_name) in [
        ("md", "Untitled.md"),
        ("rtf", "Untitled.rtf"),
        ("html", "Untitled.html"),
        ("json", "Untitled.json"),
        ("csv", "Untitled.csv"),
        ("sh", "Untitled.sh"),
        ("py", "Untitled.py"),
        ("swift", "Untitled.swift"),
    ] {
        let created = create_new_item(id, Some(dir.path()), &prefs, false).unwrap();
        let p = Path::new(&created.path);
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), expected_name);
        assert!(p.is_file());
        if id == "sh" {
            let mode = fs::metadata(p).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0, "shell script must be executable");
        }
    }

    // Create valid OOXML Office files (.docx, .xlsx, .pptx) and inspect ZIP entries
    let docx = create_new_item("docx", Some(dir.path()), &prefs, false).unwrap();
    let xlsx = create_new_item("xlsx", Some(dir.path()), &prefs, false).unwrap();
    let pptx = create_new_item("pptx", Some(dir.path()), &prefs, false).unwrap();

    let docx_zip = ZipArchive::new(File::open(&docx.path).unwrap()).unwrap();
    assert!(docx_zip.file_names().any(|n| n == "word/document.xml"));

    let xlsx_zip = ZipArchive::new(File::open(&xlsx.path).unwrap()).unwrap();
    assert!(xlsx_zip.file_names().any(|n| n == "xl/workbook.xml"));

    let pptx_zip = ZipArchive::new(File::open(&pptx.path).unwrap()).unwrap();
    assert!(pptx_zip.file_names().any(|n| n == "ppt/presentation.xml"));

    // Verify native bridge can open generated DOCX -> PDF
    let docx_pdf = dir.path().join("from_blank_docx.pdf");
    bridge::convert_doc(Path::new(&docx.path), "PDF", &docx_pdf, "a4", None).unwrap();
    assert!(fs::read(&docx_pdf).unwrap().starts_with(b"%PDF-"));

    // User Templates directory + group subfolder + placeholders
    let tmpl_root = dir.path().join("UserTemplates");
    ensure_templates_dir_initialized(&tmpl_root).unwrap();
    assert!(tmpl_root.join("README.txt").is_file());

    let group_dir = tmpl_root.join("Notes");
    fs::create_dir_all(&group_dir).unwrap();
    let note_tmpl = group_dir.join("Meeting Note.md");
    fs::write(
        &note_tmpl,
        "# Notes for {{folder}}\nDate: {{date}} {{time}}\nAuthor: {{user}}\n",
    )
    .unwrap();

    let found = scan_user_templates(&tmpl_root);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Meeting Note");
    assert_eq!(found[0].group.as_deref(), Some("Notes"));

    let target_sub = dir.path().join("ProjectAlpha");
    fs::create_dir(&target_sub).unwrap();
    let from_tmpl = create_new_item(&found[0].id, Some(&target_sub), &prefs, false).unwrap();
    let expanded = fs::read_to_string(&from_tmpl.path).unwrap();
    assert!(expanded.contains("# Notes for ProjectAlpha"));
    assert!(!expanded.contains("{{date}}") && !expanded.contains("{{time}}") && !expanded.contains("{{user}}"));

    // Read-only folder refused cleanly
    let ro_dir = dir.path().join("ReadOnlyFolder");
    fs::create_dir(&ro_dir).unwrap();
    let mut perms = fs::metadata(&ro_dir).unwrap().permissions();
    perms.set_mode(0o555);
    fs::set_permissions(&ro_dir, perms).unwrap();
    let ro_res = create_new_item("txt", Some(&ro_dir), &prefs, false);
    // Restore permissions for tempdir cleanup before asserting
    let mut restore = fs::metadata(&ro_dir).unwrap().permissions();
    restore.set_mode(0o755);
    fs::set_permissions(&ro_dir, restore).unwrap();
    assert!(ro_res.is_err(), "read-only folder must refuse file creation");
}

#[test]
fn policy_capabilities_match_detected_tools() {
    let tools = detect_tools(None);
    let caps: EngineCapabilities = tools.capabilities();
    assert!(tools.bridge.is_some(), "flip-macos-bridge must be discoverable");
    assert_eq!(caps.ffmpeg, tools.ffmpeg.is_some());
    assert_eq!(caps.pandoc, tools.pandoc.is_some());

    // Verify QuickLook thumbnail and non-destructive clipboard inspection via bridge
    let thumb = bridge::thumbnail(&repo_root().join("docs/assets/flip-cover.png"), 64).unwrap();
    assert!(thumb.ok);
    assert!(thumb.png_base64.unwrap_or_default().len() > 100);
    let _clip = bridge::clipboard_inspect().unwrap();
}

#[test]
fn output_subfolder_and_corrupt_data_edge_cases() {
    let dir = tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let prefs = AppPreferences {
        output_subfolder: Some("Converted".to_string()),
        ..Default::default()
    };

    let src = dir.path().join("item.json");
    fs::write(&src, r#"{"hello":"world"}"#).unwrap();
    let res = convert_files(&[src], &["YAML".into()], &prefs, &cancel, None).unwrap();
    assert!(res[0].error.is_none());
    assert_eq!(
        PathBuf::from(&res[0].outputs[0]),
        dir.path().join("Converted").join("item.yaml")
    );

    // Empty & malformed structured data rejected cleanly
    let empty_json = dir.path().join("empty.json");
    fs::write(&empty_json, "   ").unwrap();
    let bad_json = dir.path().join("bad.json");
    fs::write(&bad_json, "{not json").unwrap();
    let bad_res = convert_files(&[empty_json, bad_json], &["YAML".into()], &prefs, &cancel, None).unwrap();
    assert!(bad_res.iter().all(|r| r.error.is_some() && r.outputs.is_empty()));
    let _ = HashSet::<String>::new();
}

#[test]
fn regression_fixes_pdf_rotation_lock_exif_html_utf8_media_gif_cancel_and_iwork() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let tools = detect_tools(Some(&prefs));

    // 1. UTF-8 HTML without <meta charset="utf-8"> preserves accents in TXT
    let html_no_meta = dir.path().join("no_meta.html");
    fs::write(
        &html_no_meta,
        "<html><body><h1>Titolo caffè perché €</h1><p>Città italiana</p></body></html>",
    )
    .unwrap();
    let html_txt = convert_files(&[html_no_meta], &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(html_txt[0].error.is_none());
    let extracted_html_txt = fs::read_to_string(&html_txt[0].outputs[0]).unwrap();
    assert!(
        extracted_html_txt.contains("caffè") && extracted_html_txt.contains("perché") && extracted_html_txt.contains('€'),
        "HTML without meta charset lost UTF-8: {extracted_html_txt}"
    );

    // 2. MD -> DOCX -> MD preserves list items & heading markers
    let md_src = dir.path().join("struct.md");
    fs::write(&md_src, "# Structured Heading\n\nBold **caffè**.\n\n- Bullet One\n- Bullet Two\n").unwrap();
    let md_docx = convert_files(&[md_src], &["DOCX".into()], &prefs, &cancel, None).unwrap();
    assert!(md_docx[0].error.is_none());
    let docx_md = convert_files(
        &[PathBuf::from(&md_docx[0].outputs[0])],
        &["MD".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(docx_md[0].error.is_none());
    let round_md = fs::read_to_string(&docx_md[0].outputs[0]).unwrap();
    assert!(
        round_md.contains("- Bullet One") && round_md.contains("- Bullet Two"),
        "DOCX -> MD lost list bullets:\n{round_md}"
    );

    // 3. Rotated (/Rotate 90) + offset MediaBox PDF -> PNG upright & unclipped, plus Locked PDF rejection
    let rot_pdf = dir.path().join("rotated_offset.pdf");
    let locked_pdf = dir.path().join("locked.pdf");
    let swift_gen = dir.path().join("gen_pdfs.swift");
    fs::write(
        &swift_gen,
        format!(
            r#"import Foundation
import AppKit
import PDFKit
import CoreGraphics
import CoreText

let box = CGRect(x: 100, y: 100, width: 300, height: 200)
var mediaBox = box
let data = NSMutableData()
let consumer = CGDataConsumer(data: data as CFMutableData)!
let ctx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil)!
ctx.beginPDFPage(nil)
ctx.setFillColor(CGColor.white)
ctx.fill(box)
let line = CTLineCreateWithAttributedString(NSAttributedString(string: "ROTATED OFFSET TEXT", attributes: [
    .font: NSFont.boldSystemFont(ofSize: 22),
    .foregroundColor: NSColor.black
]))
ctx.textPosition = CGPoint(x: 120, y: 190)
CTLineDraw(line, ctx)
ctx.endPDFPage()
ctx.closePDF()

let doc = PDFDocument(data: data as Data)!
doc.page(at: 0)!.rotation = 90
doc.write(toFile: "{}")

let doc2 = PDFDocument(data: data as Data)!
doc2.write(toFile: "{}", withOptions: [
    PDFDocumentWriteOption.ownerPasswordOption: "secret123",
    PDFDocumentWriteOption.userPasswordOption: "secret123"
])
"#,
            rot_pdf.display(),
            locked_pdf.display()
        ),
    )
    .unwrap();
    let gen_status = Command::new("/usr/bin/swift").arg(&swift_gen).status().unwrap();
    assert!(gen_status.success());

    let rot_pngs = convert_files(&[rot_pdf], &["PNG".into()], &prefs, &cancel, None).unwrap();
    assert!(rot_pngs[0].error.is_none());
    let rot_img = image::open(&rot_pngs[0].outputs[0]).unwrap();
    assert_eq!(rot_img.dimensions(), (400, 600), "rotated PDF must render 400x600 at 144 DPI");
    let ocr_rot = bridge::ocr(Path::new(&rot_pngs[0].outputs[0]), None, None).unwrap();
    let ocr_text = ocr_rot.get("text").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        ocr_text.contains("ROTATED OFFSET TEXT"),
        "rotated PDF OCR clipped text: {ocr_text}"
    );

    let locked_png = convert_files(std::slice::from_ref(&locked_pdf), &["PNG".into()], &prefs, &cancel, None).unwrap();
    assert!(
        locked_png[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("password-protected"),
        "expected password-protected error, got: {:?}",
        locked_png[0].error
    );
    let locked_txt = convert_files(&[locked_pdf], &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(locked_txt[0]
        .error
        .as_deref()
        .unwrap_or("")
        .contains("password-protected"));

    // 4. EXIF preservation and GPS stripping through convert_files
    let exif_src = dir.path().join("with_exif.jpg");
    let swift_exif = dir.path().join("mk_exif.swift");
    fs::write(
        &swift_exif,
        format!(
            r#"import Foundation
import ImageIO
import UniformTypeIdentifiers
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, nil)!
let img = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
let props: [CFString: Any] = [
    kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFMake: "FlipCam"],
    kCGImagePropertyExifDictionary: [kCGImagePropertyExifUserComment: "FlipExif"],
    kCGImagePropertyGPSDictionary: [kCGImagePropertyGPSLatitude: 45.4642, kCGImagePropertyGPSLongitude: 9.1900]
]
CGImageDestinationAddImage(dest, img, props as CFDictionary)
CGImageDestinationFinalize(dest)
"#,
            repo_root().join("docs/assets/flip-cover.png").display(),
            exif_src.display()
        ),
    )
    .unwrap();
    assert!(Command::new("/usr/bin/swift").arg(&swift_exif).status().unwrap().success());

    let mut no_gps_prefs = prefs.clone();
    no_gps_prefs.preserve_exif = true;
    no_gps_prefs.strip_gps_exif = true;
    let jpg_nogps = convert_files(std::slice::from_ref(&exif_src), &["TIFF".into()], &no_gps_prefs, &cancel, None).unwrap();
    assert!(jpg_nogps[0].error.is_none());
    let tiff_nogps_path = PathBuf::from(&jpg_nogps[0].outputs[0]);

    let inspect_exif = dir.path().join("inspect_exif.swift");
    fs::write(
        &inspect_exif,
        format!(
            r#"import Foundation
import ImageIO
let s = CGImageSourceCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, nil)!
let p = CGImageSourceCopyPropertiesAtIndex(s, 0, nil) as? [CFString: Any] ?? [:]
print("exif=\(p[kCGImagePropertyExifDictionary] != nil) tiff=\(p[kCGImagePropertyTIFFDictionary] != nil) gps=\(p[kCGImagePropertyGPSDictionary] != nil)")
"#,
            tiff_nogps_path.display()
        ),
    )
    .unwrap();
    let exif_out = Command::new("/usr/bin/swift").arg(&inspect_exif).output().unwrap();
    let exif_stdout = String::from_utf8_lossy(&exif_out.stdout);
    assert!(
        exif_stdout.contains("exif=true") && exif_stdout.contains("tiff=true") && exif_stdout.contains("gps=false"),
        "unexpected EXIF/GPS result: {exif_stdout}"
    );

    // 5. Native MP4 -> GIF without FFmpeg, odd-dimension MP4 -> MOV, and mid-flight cancellation
    if let Some(ref ffmpeg) = tools.ffmpeg {
        let odd_mp4 = dir.path().join("odd.mp4");
        let gen_odd = Command::new(ffmpeg)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=s=321x241:d=1",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv444p",
                "-c:a",
                "aac",
                "-shortest",
                odd_mp4.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(gen_odd.status.success());

        let mov_res = convert_files(std::slice::from_ref(&odd_mp4), &["MOV".into()], &prefs, &cancel, None).unwrap();
        assert!(mov_res[0].error.is_none(), "odd dimension MP4->MOV failed: {:?}", mov_res[0].error);

        // Native GIF without FFmpeg (matching Policy::outputs for native_only!)
        let mut native_tools = tools.clone();
        native_tools.ffmpeg = None;
        let native_gif = dir.path().join("native.gif");
        media::convert(&odd_mp4, "GIF", &native_gif, &native_tools, &cancel, &|_| {}).unwrap();
        let gif_hdr = fs::read(&native_gif).unwrap();
        assert!(gif_hdr.starts_with(b"GIF8"), "native Video->GIF failed");

        // Mid-flight cancellation of running FFmpeg
        let long_wav = dir.path().join("long.wav");
        let gen_long = Command::new(ffmpeg)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=30",
                long_wav.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(gen_long.status.success());

        let cancel_mid = Arc::new(AtomicBool::new(false));
        let cancel_trigger = Arc::clone(&cancel_mid);
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            cancel_trigger.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let mid_res = convert_files(&[long_wav], &["OGG".into()], &prefs, &cancel_mid, None);
        handle.join().unwrap();
        assert!(
            matches!(mid_res, Err(EngineError::Cancelled)),
            "expected mid-flight cancellation, got {:?}",
            mid_res
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "cancellation took too long: {:?}",
            started.elapsed()
        );
    }

    // 6. iWork Create New explicit handling + template symlink refusal
    if !Path::new("/Applications/Pages.app").exists() {
        let pages_err = create_new_item("pages", Some(dir.path()), &prefs, false).unwrap_err();
        assert!(matches!(pages_err, EngineError::Unsupported(_)));
    }
    let real_tmpl = dir.path().join("real.txt");
    fs::write(&real_tmpl, "hi").unwrap();
    let link_tmpl = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real_tmpl, &link_tmpl).unwrap();
    let tmpl_err = create_new_item(&format!("template:{}", link_tmpl.display()), Some(dir.path()), &prefs, false);
    assert!(tmpl_err.is_err(), "symlinked template must be refused");
}

#[test]
fn deep_audit_regressions_a4_pagination_predecode_40mp_ocr_exif_empty_ocr_images_pdf_and_html_lfi() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    // 1. Multi-page A4 & Letter PDF pagination (>= 3 pages for 180 lines, real A4/Letter page sizes)
    let txt_180 = dir.path().join("lines180.txt");
    let mut lines_body = String::new();
    for i in 1..=180 {
        lines_body.push_str(&format!("Line {i:03} — local macOS CoreText A4 pagination check.\n"));
    }
    fs::write(&txt_180, &lines_body).unwrap();

    let a4_pdf = dir.path().join("lines180_a4.pdf");
    let a4_info = bridge::convert_doc(&txt_180, "PDF", &a4_pdf, "a4", None).unwrap();
    let a4_pages = a4_info.get("pages").and_then(|v| v.as_u64()).unwrap_or(0);
    assert!(a4_pages >= 3, "expected >= 3 A4 pages for 180 lines, got {a4_pages}");

    let letter_pdf = dir.path().join("lines180_letter.pdf");
    let letter_info = bridge::convert_doc(&txt_180, "PDF", &letter_pdf, "letter", None).unwrap();
    let letter_pages = letter_info.get("pages").and_then(|v| v.as_u64()).unwrap_or(0);
    assert!(letter_pages >= 3, "expected >= 3 Letter pages for 180 lines, got {letter_pages}");

    // 2. Pre-decode > 40 MP guard in convertImage and imagesToPDF (7001x6000 = 42,006,000 px)
    let big_png = dir.path().join("over40mp.png");
    {
        // Minimal 1-bit grayscale 7001x6000 PNG via raw image encoder (fast & tiny on disk/RAM)
        let img = image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::new(7001, 6000);
        img.save_with_format(&big_png, ImageFormat::Png).unwrap();
    }
    let big_out = dir.path().join("over40mp_out.jpg");
    let big_img_err = bridge::convert_image(&big_png, "JPG", &big_out, 0.9, true, false, None).unwrap_err();
    assert!(
        big_img_err.to_string().contains("40 megapixel"),
        "expected 40 megapixel pre-decode error, got: {big_img_err}"
    );
    assert!(!big_out.exists());

    let big_pdf_out = dir.path().join("over40mp.pdf");
    let big_pdf_err = bridge::convert_pdf_from_images(&big_pdf_out, &[big_png], None).unwrap_err();
    assert!(
        big_pdf_err.to_string().contains("40 megapixel"),
        "expected 40 megapixel error in imagesToPDF, got: {big_pdf_err}"
    );
    assert!(!big_pdf_out.exists());

    // 3. OCR EXIF orientation (Orientation 6) + refuse 0-byte TXT on empty OCR (both image and PDF)
    let rotated_ocr_jpg = dir.path().join("ocr_exif6.jpg");
    let rotated_img_400x200 = dir.path().join("ocr_plain_upright.png");
    let swift_ocr_fixture = dir.path().join("mk_ocr_exif.swift");
    fs::write(
        &swift_ocr_fixture,
        format!(
            r#"import Foundation
import AppKit
import CoreGraphics
import CoreText
import ImageIO
import UniformTypeIdentifiers

// Draw crisp upright text into 420x140 context
let w = 420, h = 140
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
ctx.setFillColor(CGColor.white)
ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
let str = NSAttributedString(string: "FLIP OCR ORIENTED", attributes: [
    .font: NSFont.boldSystemFont(ofSize: 32),
    .foregroundColor: NSColor.black
])
let line = CTLineCreateWithAttributedString(str)
ctx.textPosition = CGPoint(x: 24, y: 56)
CTLineDraw(line, ctx)
let upright = ctx.makeImage()!

let uprightDest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(uprightDest, upright, nil)
CGImageDestinationFinalize(uprightDest)

// Store rotated pixels (90 deg CCW = 140x420) with EXIF Orientation = 6 (90 deg CW to display upright)
let rotCtx = CGContext(data: nil, width: h, height: w, bitsPerComponent: 8, bytesPerRow: 0, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
rotCtx.translateBy(x: CGFloat(h), y: 0)
rotCtx.rotate(by: .pi / 2)
rotCtx.draw(upright, in: CGRect(x: 0, y: 0, width: w, height: h))
let storedRotated = rotCtx.makeImage()!

let jpgDest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(jpgDest, storedRotated, [kCGImagePropertyOrientation: 6] as CFDictionary)
CGImageDestinationFinalize(jpgDest)
"#,
            rotated_img_400x200.display(),
            rotated_ocr_jpg.display()
        ),
    )
    .unwrap();
    assert!(Command::new("/usr/bin/swift").arg(&swift_ocr_fixture).status().unwrap().success());

    let ocr_out_txt = dir.path().join("ocr_exif6.txt");
    let ocr_res = bridge::ocr(&rotated_ocr_jpg, Some(&ocr_out_txt), None).unwrap();
    let ocr_text = ocr_res.get("text").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        ocr_text.contains("FLIP") && ocr_text.contains("ORIENTED"),
        "OCR missed EXIF-oriented text: {ocr_text}"
    );
    assert!(fs::metadata(&ocr_out_txt).unwrap().len() > 0);

    // Empty OCR must fail and refuse writing a 0-byte TXT file
    let blank_img = dir.path().join("blank_shape.png");
    make_transparent_test_png(&blank_img);
    let blank_ocr_txt = dir.path().join("blank_ocr.txt");
    let blank_ocr_err = bridge::ocr(&blank_img, Some(&blank_ocr_txt), None).unwrap_err();
    assert!(blank_ocr_err.to_string().contains("OCR"));
    assert!(!blank_ocr_txt.exists(), "0-byte TXT must NOT be written on empty OCR");

    let blank_pdf = dir.path().join("blank_shape.pdf");
    bridge::convert_pdf_from_images(&blank_pdf, std::slice::from_ref(&blank_img), None).unwrap();
    let blank_pdf_txt = dir.path().join("blank_pdf.txt");
    let blank_pdf_err = bridge::convert_pdf_extract_text(&blank_pdf, &blank_pdf_txt, None).unwrap_err();
    assert!(blank_pdf_err.to_string().contains("OCR"));
    assert!(!blank_pdf_txt.exists(), "0-byte TXT must NOT be written on empty PDF OCR");

    // 4. imagesToPDF A4 595x842 layout, >128 guard, and no-overwrite guard
    let combined_a4 = dir.path().join("combined_a4.pdf");
    bridge::convert_pdf_from_images(&combined_a4, &[rotated_ocr_jpg, blank_img.clone()], None).unwrap();
    // Overwrite refusal
    assert!(bridge::convert_pdf_from_images(&combined_a4, std::slice::from_ref(&blank_img), None).is_err());
    // >128 file refusal
    let too_many_imgs: Vec<PathBuf> = vec![blank_img; 129];
    assert!(bridge::convert_pdf_from_images(&dir.path().join("too_many.pdf"), &too_many_imgs, None).is_err());

    // Inspect PDFKit bounds of a4_pdf, letter_pdf, and combined_a4
    let inspect_pdfs = dir.path().join("inspect_pdfs.swift");
    fs::write(
        &inspect_pdfs,
        format!(
            r#"import Foundation
import PDFKit
func check(_ path: String, _ expectW: CGFloat, _ expectH: CGFloat, _ minPages: Int) {{
    let doc = PDFDocument(url: URL(fileURLWithPath: path))!
    assert(doc.pageCount >= minPages, "pageCount \(doc.pageCount) < \(minPages) for \(path)")
    for i in 0..<doc.pageCount {{
        let b = doc.page(at: i)!.bounds(for: .mediaBox)
        assert(abs(b.width - expectW) < 1.0 && abs(b.height - expectH) < 1.0, "bad mediaBox \(b) for \(path)")
    }}
}}
check("{}", 595.28, 841.89, 3)
check("{}", 612.0, 792.0, 3)
check("{}", 595.0, 842.0, 2)
print("PDF_LAYOUT_OK")
"#,
            a4_pdf.display(),
            letter_pdf.display(),
            combined_a4.display()
        ),
    )
    .unwrap();
    let pdf_inspect_out = Command::new("/usr/bin/swift").arg(&inspect_pdfs).output().unwrap();
    assert!(
        pdf_inspect_out.status.success() && String::from_utf8_lossy(&pdf_inspect_out.stdout).contains("PDF_LAYOUT_OK"),
        "PDF layout check failed: stderr={}",
        String::from_utf8_lossy(&pdf_inspect_out.stderr)
    );

    // 5. HTML local file inclusion (LFI) blocked during import
    let secret_file = dir.path().join("local_secret.txt");
    fs::write(&secret_file, "SECRET_LOCAL_TOKEN_9911").unwrap();
    let lfi_html = dir.path().join("lfi_probe.html");
    fs::write(
        &lfi_html,
        format!(
            "<!doctype html><html><head><link rel=\"stylesheet\" href=\"file://{}\"></head>\
             <body><h1>Safe Title</h1><iframe src=\"file://{}\"></iframe>\
             <img src=\"file://{}\"><p>Clean paragraph.</p></body></html>",
            secret_file.display(),
            secret_file.display(),
            secret_file.display()
        ),
    )
    .unwrap();
    let lfi_res = convert_files(&[lfi_html], &["TXT".into(), "PDF".into()], &prefs, &cancel, None).unwrap();
    for r in &lfi_res {
        assert!(r.error.is_none(), "HTML sanitize failed for {}: {:?}", r.format, r.error);
    }
    let lfi_txt = fs::read_to_string(&lfi_res.iter().find(|r| r.format == "TXT").unwrap().outputs[0]).unwrap();
    assert!(lfi_txt.contains("Safe Title") && lfi_txt.contains("Clean paragraph."));
    assert!(!lfi_txt.contains("SECRET_LOCAL_TOKEN_9911"), "HTML LFI leaked local file: {lfi_txt}");

    // 6. Permissions & iWork package bundle policy check
    let perms = bridge::permissions(false, false, false).unwrap();
    assert!(!perms.automation_finder.is_empty());

    let pkg_pages = dir.path().join("bundle.pages");
    fs::create_dir_all(pkg_pages.join("QuickLook")).unwrap();
    // Copy valid PDF as Preview.pdf inside package-format .pages bundle
    fs::copy(&combined_a4, pkg_pages.join("QuickLook/Preview.pdf")).unwrap();
    let pkg_res = convert_files(&[pkg_pages], &["PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(pkg_res[0].error.is_none(), "package .pages -> PDF failed: {:?}", pkg_res[0].error);
    assert!(fs::read(&pkg_res[0].outputs[0]).unwrap().starts_with(b"%PDF-"));

    // 7. Regression tests from Step 5 skeptical review:
    // 7a. 144 DPI A4 document PNG OCR 2x retry succeeds on crisp 11.5pt body text
    let a4_pages_dir = dir.path().join("a4_144_pages");
    let rendered_144 = bridge::convert_pdf_render(&a4_pdf, &a4_pages_dir, "PNG", 144, None).unwrap();
    assert!(rendered_144.len() >= 3);
    let ocr_144 = bridge::ocr(&rendered_144[0], None, None).unwrap();
    let ocr_144_txt = ocr_144.get("text").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        ocr_144_txt.contains("Line 001") || ocr_144_txt.contains("CoreText"),
        "144 DPI A4 OCR failed: {ocr_144_txt}"
    );

    // 7b. HTML @import without semicolon does NOT swallow </style>/body, and huge font does NOT truncate trailing text
    let import_html = dir.path().join("no_semi_import.html");
    fs::write(
        &import_html,
        format!(
            "<html><head><style>@import 'file://{}'</style></head>\
             <body><p srcset=\"file://{}\">VISIBLE_AFTER_IMPORT</p></body></html>",
            secret_file.display(),
            secret_file.display()
        ),
    )
    .unwrap();
    let imp_res = convert_files(&[import_html], &["TXT".into(), "PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(imp_res.iter().all(|r| r.error.is_none()));
    let imp_txt = fs::read_to_string(&imp_res.iter().find(|r| r.format == "TXT").unwrap().outputs[0]).unwrap();
    assert!(imp_txt.contains("VISIBLE_AFTER_IMPORT"), "HTML @import ate body: {imp_txt}");

    let huge_glyph_html = dir.path().join("huge_glyph.html");
    fs::write(
        &huge_glyph_html,
        "<html><body><p style=\"font-size:900pt\">X</p><p>AFTER_HUGE_GLYPH</p></body></html>",
    )
    .unwrap();
    let huge_pdf_res = convert_files(&[huge_glyph_html], &["PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(huge_pdf_res[0].error.is_none());
    let huge_extracted = dir.path().join("huge_extracted.txt");
    bridge::convert_pdf_extract_text(Path::new(&huge_pdf_res[0].outputs[0]), &huge_extracted, None).unwrap();
    let huge_txt = fs::read_to_string(&huge_extracted).unwrap();
    assert!(huge_txt.contains("AFTER_HUGE_GLYPH"), "huge glyph truncated PDF text: {huge_txt}");

    // 7c. > 40 MP image -> PDF through convert_files rejects cleanly without fallback decode
    let over40_pdf_res = convert_files(&[dir.path().join("over40mp.png")], &["PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(
        over40_pdf_res[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("40 megapixel"),
        "expected 40 megapixel error, got: {:?}",
        over40_pdf_res[0].error
    );

    // 7d. iWork zip bomb (> 256 MiB QuickLook/Preview.pdf) rejected before extraction
    let pages_bomb = dir.path().join("bomb.pages");
    {
        let file = File::create(&pages_bomb).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("QuickLook/Preview.pdf", opts).unwrap();
        let zeros = vec![0u8; 1024 * 1024];
        for _ in 0..257 {
            zip.write_all(&zeros).unwrap();
        }
        zip.finish().unwrap();
    }
    let pages_bomb_res = convert_files(&[pages_bomb], &["PDF".into()], &prefs, &cancel, None).unwrap();
    assert!(
        pages_bomb_res[0].error.as_deref().unwrap_or("").contains("256 MiB"),
        "expected 256 MiB error for bomb.pages, got: {:?}",
        pages_bomb_res[0].error
    );

    // 7e. clipboard-save refuses overwriting an existing file
    let existing_clip = dir.path().join("keep_clip.txt");
    fs::write(&existing_clip, b"KEEP_ME_SAFE").unwrap();
    let clip_save_res = bridge::clipboard_save(&existing_clip);
    assert!(clip_save_res.is_err(), "clipboard_save must refuse overwriting existing file");
    assert_eq!(fs::read(&existing_clip).unwrap(), b"KEEP_ME_SAFE");
}

struct RestoreDirPermissions(PathBuf);

impl Drop for RestoreDirPermissions {
    fn drop(&mut self) {
        if let Ok(meta) = fs::metadata(&self.0) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&self.0, perms);
        }
    }
}

#[test]
fn readonly_source_directory_falls_back_to_fallback_folder() {
    let dir = tempdir().unwrap();
    let ro_dir = dir.path().join("ReadOnlyMount");
    fs::create_dir(&ro_dir).unwrap();

    let ro_png = ro_dir.join("mounted.png");
    let ro_json = ro_dir.join("mounted.json");
    make_transparent_test_png(&ro_png);
    fs::write(&ro_json, r#"{"readonly":true,"count":42}"#).unwrap();
    for ro_file in [&ro_png, &ro_json] {
        let mut fperms = fs::metadata(ro_file).unwrap().permissions();
        fperms.set_mode(0o444);
        fs::set_permissions(ro_file, fperms).unwrap();
    }

    let png_hash = sha256_file(&ro_png);
    let json_hash = sha256_file(&ro_json);

    let _ro_guard = RestoreDirPermissions(ro_dir.clone());
    let mut ro_perms = fs::metadata(&ro_dir).unwrap().permissions();
    ro_perms.set_mode(0o555);
    fs::set_permissions(&ro_dir, ro_perms).unwrap();

    // Non-existent nested fallback_folder should be created automatically
    let fallback_dir = dir.path().join("FallbackOut/Nested");
    let prefs = AppPreferences {
        fallback_folder: Some(fallback_dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));

    let img_res = convert_files(
        std::slice::from_ref(&ro_png),
        &["JPG".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(img_res[0].error.is_none(), "ro png->jpg error: {:?}", img_res[0].error);
    let jpg_out = PathBuf::from(&img_res[0].outputs[0]);
    assert_eq!(jpg_out.parent(), Some(fallback_dir.as_path()));
    assert_eq!(jpg_out.file_name().unwrap().to_str().unwrap(), "mounted.jpg");
    assert!(jpg_out.is_file());
    assert!(!fs::metadata(&jpg_out).unwrap().permissions().readonly());

    // Even with output_subfolder configured, a read-only source directory falls back cleanly
    let prefs_with_sub = AppPreferences {
        fallback_folder: Some(fallback_dir.to_string_lossy().into_owned()),
        output_subfolder: Some("Converted".to_string()),
        ..Default::default()
    };
    let data_res = convert_files(
        std::slice::from_ref(&ro_json),
        &["YAML".into()],
        &prefs_with_sub,
        &cancel,
        None,
    )
    .unwrap();
    assert!(data_res[0].error.is_none(), "ro json->yaml error: {:?}", data_res[0].error);
    let yaml_out = PathBuf::from(&data_res[0].outputs[0]);
    assert_eq!(yaml_out.parent(), Some(fallback_dir.as_path()));
    assert_eq!(yaml_out.file_name().unwrap().to_str().unwrap(), "mounted.yaml");
    assert!(fs::read_to_string(&yaml_out).unwrap().contains("readonly: true"));

    // Combine-PDF batch action from read-only folder also lands in fallback_folder
    let ro_png2 = dir.path().join("writable.png");
    make_transparent_test_png(&ro_png2);
    let merge_res = convert_files(
        &[ro_png.clone(), ro_png2],
        &["COMBINE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(merge_res[0].error.is_none(), "ro combine error: {:?}", merge_res[0].error);
    let comb_out = PathBuf::from(&merge_res[0].outputs[0]);
    assert_eq!(comb_out.parent(), Some(fallback_dir.as_path()));
    assert!(fs::read(&comb_out).unwrap().starts_with(b"%PDF-"));

    // Read-only source folder must remain completely untouched
    let mut ro_entries: Vec<String> = fs::read_dir(&ro_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ro_entries.sort();
    assert_eq!(ro_entries, vec!["mounted.json", "mounted.png"]);
    assert_eq!(sha256_file(&ro_png), png_hash);
    assert_eq!(sha256_file(&ro_json), json_hash);

    // If fallback_folder itself is also read-only, resolve_target_dir falls back to ~/Downloads
    let ro_fallback = dir.path().join("ReadOnlyFallback");
    fs::create_dir(&ro_fallback).unwrap();
    let _fb_guard = RestoreDirPermissions(ro_fallback.clone());
    let mut fb_perms = fs::metadata(&ro_fallback).unwrap().permissions();
    fb_perms.set_mode(0o555);
    fs::set_permissions(&ro_fallback, fb_perms).unwrap();

    let prefs_ro_fb = AppPreferences {
        fallback_folder: Some(ro_fallback.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let resolved_secondary = resolve_target_dir(&ro_png, &prefs_ro_fb).unwrap();
    let expected_downloads = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
    });
    assert_eq!(resolved_secondary, expected_downloads);
}

fn assert_valid_epub3_archive(epub_path: &Path) {
    use std::io::Read;
    let file = File::open(epub_path).expect("open generated epub");
    let mut archive = ZipArchive::new(file).expect("parse epub zip");
    assert!(archive.len() >= 6, "expected >= 6 EPUB 3 container entries");

    {
        let mut entry0 = archive.by_index(0).expect("entry 0 mimetype");
        assert_eq!(entry0.name(), "mimetype", "EPUB 3 requires mimetype as entry 0");
        assert_eq!(
            entry0.compression(),
            CompressionMethod::Stored,
            "EPUB 3 requires mimetype stored uncompressed"
        );
        let mut mime = String::new();
        entry0.read_to_string(&mut mime).unwrap();
        assert_eq!(mime, "application/epub+zip");
    }

    for required in [
        "META-INF/container.xml",
        "OEBPS/content.opf",
        "OEBPS/nav.xhtml",
        "OEBPS/style.css",
        "OEBPS/chapter.xhtml",
    ] {
        assert!(
            archive.by_name(required).is_ok(),
            "EPUB 3 missing required entry `{required}`"
        );
    }

    let mut opf = String::new();
    archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
    assert!(opf.contains("dc:identifier") && opf.contains("dc:title") && opf.contains("dc:language"));
    assert!(opf.contains("dcterms:modified"));

    let mut nav = String::new();
    archive.by_name("OEBPS/nav.xhtml").unwrap().read_to_string(&mut nav).unwrap();
    assert!(nav.contains("epub:type=\"toc\""));
}

#[test]
fn native_epub3_md_epub_txt_and_pdf_epub_roundtrips_without_pandoc() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    // 1. MD -> EPUB (zero Pandoc) + mimetype Stored entry 0 verification
    let md_src = dir.path().join("handbook.md");
    fs::write(
        &md_src,
        "# FLIP Native EPUB\n\nEspresso & **caffè** guide.\n\n- Alpha chapter\n- Beta chapter\n",
    )
    .unwrap();
    let md_hash = sha256_file(&md_src);

    let md_to_epub = convert_files(std::slice::from_ref(&md_src), &["EPUB".into()], &prefs, &cancel, None).unwrap();
    assert!(md_to_epub[0].error.is_none(), "MD -> EPUB failed: {:?}", md_to_epub[0].error);
    let epub_from_md = PathBuf::from(&md_to_epub[0].outputs[0]);
    assert_valid_epub3_archive(&epub_from_md);

    // 2. EPUB -> all 11 native target formats (TXT, MD, HTML, RTF, DOCX, ODT, PDF, PNG, JPG, WEBP, HEIC)
    let epub_targets: Vec<String> = [
        "TXT", "MD", "HTML", "RTF", "DOCX", "ODT", "PDF", "PNG", "JPG", "WEBP", "HEIC",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let epub_outs = convert_files(std::slice::from_ref(&epub_from_md), &epub_targets, &prefs, &cancel, None).unwrap();
    for r in &epub_outs {
        assert!(r.error.is_none(), "EPUB -> {} failed: {:?}", r.format, r.error);
        assert!(!r.outputs.is_empty(), "EPUB -> {} produced 0 outputs", r.format);
    }

    let epub_txt = fs::read_to_string(&epub_outs.iter().find(|r| r.format == "TXT").unwrap().outputs[0]).unwrap();
    assert!(
        epub_txt.contains("FLIP Native EPUB") && epub_txt.contains("caffè") && epub_txt.contains("Beta chapter"),
        "EPUB -> TXT lost content:\n{epub_txt}"
    );
    let epub_md = fs::read_to_string(&epub_outs.iter().find(|r| r.format == "MD").unwrap().outputs[0]).unwrap();
    assert!(epub_md.contains("# FLIP Native EPUB") && epub_md.contains("caffè"));
    let epub_pdf = PathBuf::from(&epub_outs.iter().find(|r| r.format == "PDF").unwrap().outputs[0]);
    assert!(fs::read(&epub_pdf).unwrap().starts_with(b"%PDF-"));

    // 3. PDF -> EPUB -> TXT roundtrip (zero Pandoc)
    let pdf_to_epub = convert_files(std::slice::from_ref(&epub_pdf), &["EPUB".into()], &prefs, &cancel, None).unwrap();
    assert!(pdf_to_epub[0].error.is_none(), "PDF -> EPUB failed: {:?}", pdf_to_epub[0].error);
    let epub_from_pdf = PathBuf::from(&pdf_to_epub[0].outputs[0]);
    assert_valid_epub3_archive(&epub_from_pdf);

    let pdf_epub_txt = convert_files(&[epub_from_pdf], &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(pdf_epub_txt[0].error.is_none(), "PDF->EPUB->TXT failed: {:?}", pdf_epub_txt[0].error);
    let roundtrip_txt = fs::read_to_string(&pdf_epub_txt[0].outputs[0]).unwrap();
    assert!(
        roundtrip_txt.contains("FLIP Native EPUB") && roundtrip_txt.contains("caffè"),
        "PDF -> EPUB -> TXT lost text:\n{roundtrip_txt}"
    );

    // 4. DOCX / ODT / RTF / TXT / HTML -> EPUB roundtrips
    for fmt in ["TXT", "HTML", "DOCX", "RTF", "ODT"] {
        let mid_path = PathBuf::from(&epub_outs.iter().find(|r| r.format == fmt).unwrap().outputs[0]);
        let to_epub = convert_files(&[mid_path], &["EPUB".into()], &prefs, &cancel, None).unwrap();
        assert!(to_epub[0].error.is_none(), "{fmt} -> EPUB failed: {:?}", to_epub[0].error);
        let gen_epub = PathBuf::from(&to_epub[0].outputs[0]);
        assert_valid_epub3_archive(&gen_epub);
        let back_txt = convert_files(&[gen_epub], &["TXT".into()], &prefs, &cancel, None).unwrap();
        assert!(back_txt[0].error.is_none(), "{fmt}->EPUB->TXT failed: {:?}", back_txt[0].error);
        let text = fs::read_to_string(&back_txt[0].outputs[0]).unwrap();
        assert!(text.contains("caffè"), "{fmt} -> EPUB -> TXT lost caffè:\n{text}");
    }

    assert_eq!(sha256_file(&md_src), md_hash);
}

fn assert_no_residual_staging(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".flip-job-"),
            "Residual staging directory leaked in {}: {name}",
            root.display()
        );
    }
}

fn build_test_pdf(path: &Path, page_count: usize, embed_rgb_image: bool) {
    // Minimal valid PDF 1.4 generator with Helvetica font, selectable text per page,
    // and optional uncompressed 500x360 DeviceRGB raster stream (~540 KB) for OPTIMIZE_PDF tests.
    let mut pdf = Vec::<u8>::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");

    let mut offsets = Vec::<usize>::new();
    let img_obj_id = 4usize;
    let first_page_id = if embed_rgb_image { 5 } else { 4 };

    // 1: Catalog
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // 2: Pages
    let kids: String = (0..page_count)
        .map(|i| format!("{} 0 R", first_page_id + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {page_count} >>\nendobj\n")
            .as_bytes(),
    );

    // 3: Font
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );

    // 4: Optional large uncompressed RGB Image XObject
    if embed_rgb_image {
        let w = 500usize;
        let h = 360usize;
        let mut raw_rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                raw_rgb.push(((x * 255) / w) as u8);
                raw_rgb.push(((y * 255) / h) as u8);
                raw_rgb.push((((x ^ y) * 3) & 0xFF) as u8);
            }
        }
        offsets.push(pdf.len());
        pdf.extend_from_slice(
            format!(
                "{img_obj_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                raw_rgb.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&raw_rgb);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xobj_res = if embed_rgb_image {
        format!(" /XObject << /Im1 {img_obj_id} 0 R >>")
    } else {
        String::new()
    };

    for i in 0..page_count {
        let page_id = first_page_id + i * 2;
        let content_id = page_id + 1;
        offsets.push(pdf.len());
        pdf.extend_from_slice(
            format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >>{xobj_res} >> /Contents {content_id} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );

        let stream_body = if embed_rgb_image {
            format!(
                "q 500 0 0 360 47 220 cm /Im1 Do Q\nBT /F1 18 Tf 72 720 Td (FLIP Selectable Marker Page {}) Tj ET\n",
                i + 1
            )
        } else {
            format!("BT /F1 14 Tf 72 720 Td (Page {}) Tj ET\n", i + 1)
        };
        offsets.push(pdf.len());
        pdf.extend_from_slice(
            format!(
                "{content_id} 0 obj\n<< /Length {} >>\nstream\n{stream_body}endstream\nendobj\n",
                stream_body.len()
            )
            .as_bytes(),
        );
    }

    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );

    fs::write(path, &pdf).expect("write generated pdf");
}

#[test]
fn zip_and_unzip_roundtrip_dedup_collisions_and_safety_rejections() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    // 1. Single-file ZIP + non-overwrite collision (`note.zip` -> `note 2.zip`)
    let note = dir.path().join("note.txt");
    fs::write(&note, b"single zip payload").unwrap();
    let note_hash = sha256_file(&note);

    let single_zip_res =
        convert_files(std::slice::from_ref(&note), &["ZIP".into()], &prefs, &cancel, None).unwrap();
    assert!(single_zip_res[0].error.is_none(), "{:?}", single_zip_res[0].error);
    let first_zip = PathBuf::from(&single_zip_res[0].outputs[0]);
    assert_eq!(first_zip.file_name().unwrap().to_str().unwrap(), "note.zip");
    let first_zip_hash = sha256_file(&first_zip);

    let second_zip_res =
        convert_files(std::slice::from_ref(&note), &["ZIP".into()], &prefs, &cancel, None).unwrap();
    assert!(second_zip_res[0].error.is_none(), "{:?}", second_zip_res[0].error);
    let second_zip = PathBuf::from(&second_zip_res[0].outputs[0]);
    assert_eq!(second_zip.file_name().unwrap().to_str().unwrap(), "note 2.zip");
    assert_eq!(sha256_file(&first_zip), first_zip_hash);
    assert_eq!(sha256_file(&note), note_hash);

    // 2. Single .zip refused for ZIP (must use UNZIP)
    let double_zip_err =
        convert_files(std::slice::from_ref(&first_zip), &["ZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(double_zip_err[0].error.is_some());

    // 3. Multi-file ZIP across distinct folders with colliding basenames -> `report.txt` & `report (2).txt`
    let folder_a = dir.path().join("folder_a");
    let folder_b = dir.path().join("folder_b");
    let folder_c = dir.path().join("folder_c");
    fs::create_dir_all(&folder_a).unwrap();
    fs::create_dir_all(&folder_b).unwrap();
    fs::create_dir_all(&folder_c).unwrap();
    let r_a = folder_a.join("report.txt");
    let r_b = folder_b.join("report.txt");
    let pic_c = folder_c.join("photo.png");
    fs::write(&r_a, b"alpha report").unwrap();
    fs::write(&r_b, b"beta report").unwrap();
    make_transparent_test_png(&pic_c);

    let batch_zip_res = convert_files(
        &[r_a.clone(), r_b.clone(), pic_c.clone()],
        &["ZIP".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(batch_zip_res[0].error.is_none(), "{:?}", batch_zip_res[0].error);
    let batch_zip = PathBuf::from(&batch_zip_res[0].outputs[0]);
    assert_eq!(batch_zip.file_name().unwrap().to_str().unwrap(), "report.zip");

    let mut zr = ZipArchive::new(File::open(&batch_zip).unwrap()).unwrap();
    assert_eq!(zr.len(), 3);
    let mut entry_contents = HashMap::new();
    for idx in 0..zr.len() {
        let mut e = zr.by_index(idx).unwrap();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        entry_contents.insert(e.name().to_string(), data);
    }
    assert_eq!(entry_contents.get("report.txt").map(Vec::as_slice), Some(b"alpha report".as_slice()));
    assert_eq!(entry_contents.get("report (2).txt").map(Vec::as_slice), Some(b"beta report".as_slice()));
    assert!(entry_contents.contains_key("photo.png"));

    // 4. UNZIP nested folder with executable permissions + sibling collision (`bundle` & `bundle 2`)
    let bundle_zip = dir.path().join("bundle.zip");
    {
        let mut zw = ZipWriter::new(File::create(&bundle_zip).unwrap());
        zw.add_directory(
            "docs/",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
        zw.start_file(
            "docs/readme.txt",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zw.write_all(b"Hello bundle").unwrap();
        zw.start_file(
            "bin/run.sh",
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .unix_permissions(0o755),
        )
        .unwrap();
        zw.write_all(b"#!/bin/sh\necho ok\n").unwrap();
        zw.finish().unwrap();
    }

    let unzip1 =
        convert_files(std::slice::from_ref(&bundle_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(unzip1[0].error.is_none(), "{:?}", unzip1[0].error);
    let out_dir1 = PathBuf::from(&unzip1[0].outputs[0]);
    assert_eq!(out_dir1, dir.path().join("bundle"));
    assert_eq!(
        fs::read_to_string(out_dir1.join("docs/readme.txt")).unwrap(),
        "Hello bundle"
    );
    let script_mode = fs::metadata(out_dir1.join("bin/run.sh")).unwrap().permissions().mode();
    assert_ne!(script_mode & 0o111, 0, "executable bit should be preserved");

    let unzip2 =
        convert_files(std::slice::from_ref(&bundle_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(unzip2[0].error.is_none(), "{:?}", unzip2[0].error);
    let out_dir2 = PathBuf::from(&unzip2[0].outputs[0]);
    assert_eq!(out_dir2, dir.path().join("bundle 2"));
    assert_eq!(
        fs::read_to_string(out_dir1.join("docs/readme.txt")).unwrap(),
        "Hello bundle"
    );
    assert_eq!(
        fs::read_to_string(out_dir2.join("docs/readme.txt")).unwrap(),
        "Hello bundle"
    );

    // 5. Reject zip-slip traversal (`../escaped.txt` and `/tmp/abs.txt`)
    let slip_zip = dir.path().join("slip.zip");
    {
        let mut zw = ZipWriter::new(File::create(&slip_zip).unwrap());
        zw.start_file("../escaped.txt", SimpleFileOptions::default()).unwrap();
        zw.write_all(b"pwned").unwrap();
        zw.finish().unwrap();
    }
    let slip_res =
        convert_files(std::slice::from_ref(&slip_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(slip_res[0].error.is_some());
    assert!(!dir.path().join("escaped.txt").exists());
    assert!(!dir.path().join("slip").exists());

    let abs_zip = dir.path().join("abs.zip");
    {
        let mut zw = ZipWriter::new(File::create(&abs_zip).unwrap());
        zw.start_file("/tmp/abs_escaped.txt", SimpleFileOptions::default()).unwrap();
        zw.write_all(b"pwned").unwrap();
        zw.finish().unwrap();
    }
    let abs_res =
        convert_files(std::slice::from_ref(&abs_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(abs_res[0].error.is_some());

    // 6. Reject Unix symlink entry (`S_IFLNK = 0o120000`)
    let link_zip = dir.path().join("symlink.zip");
    {
        let mut zw = ZipWriter::new(File::create(&link_zip).unwrap());
        zw.add_symlink("leak", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        zw.finish().unwrap();
    }
    let link_res =
        convert_files(std::slice::from_ref(&link_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(link_res[0].error.is_some());
    assert!(!dir.path().join("symlink").exists());

    // 7. Reject > MAX_CONTAINER_ENTRIES (4,096)
    let many_zip = dir.path().join("many.zip");
    {
        let mut zw = ZipWriter::new(File::create(&many_zip).unwrap());
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for i in 0..=MAX_CONTAINER_ENTRIES {
            zw.start_file(format!("e{i}.txt"), opts).unwrap();
        }
        zw.finish().unwrap();
    }
    let many_res =
        convert_files(std::slice::from_ref(&many_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(many_res[0].error.is_some());
    assert!(!dir.path().join("many").exists());

    // 8. Reject cumulative > MAX_CONTAINER_EXPANDED_BYTES (256 MiB) compression bomb
    let bomb_zip = dir.path().join("bomb.zip");
    {
        let mut zw = ZipWriter::new(File::create(&bomb_zip).unwrap());
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let chunk = vec![0u8; 1024 * 1024];
        let half_limit_mib = (MAX_CONTAINER_EXPANDED_BYTES as usize / (1024 * 1024)) / 2 + 8;
        for part in ["part_a.bin", "part_b.bin"] {
            zw.start_file(part, opts).unwrap();
            for _ in 0..half_limit_mib {
                zw.write_all(&chunk).unwrap();
            }
        }
        zw.finish().unwrap();
    }
    let bomb_res =
        convert_files(std::slice::from_ref(&bomb_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(bomb_res[0].error.is_some());
    assert!(!dir.path().join("bomb").exists());

    // 8b. Also verify forged small header uncompressed_size cannot bypass cumulative actual-byte check
    let mut forged = fs::read(&bomb_zip).unwrap();
    let half_bytes = (((MAX_CONTAINER_EXPANDED_BYTES as usize / (1024 * 1024)) / 2 + 8) * 1024 * 1024) as u32;
    let half_le = half_bytes.to_le_bytes();
    let forged_le = 16u32.to_le_bytes();
    let mut patched_headers = 0usize;
    for idx in 0..forged.len().saturating_sub(30) {
        if &forged[idx..idx + 4] == b"PK\x03\x04" && forged[idx + 22..idx + 26] == half_le {
            forged[idx + 22..idx + 26].copy_from_slice(&forged_le);
            patched_headers += 1;
        } else if &forged[idx..idx + 4] == b"PK\x01\x02" && forged[idx + 24..idx + 28] == half_le {
            forged[idx + 24..idx + 28].copy_from_slice(&forged_le);
            patched_headers += 1;
        }
    }
    assert_eq!(patched_headers, 4, "expected 2 local + 2 central headers");
    let forged_zip = dir.path().join("forged_bomb.zip");
    fs::write(&forged_zip, &forged).unwrap();
    let forged_res =
        convert_files(std::slice::from_ref(&forged_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    let forged_err = forged_res[0].error.as_deref().unwrap_or("");
    assert!(
        forged_err.contains("256 MiB"),
        "expected 256 MiB bomb error for forged header bomb, got: {forged_err:?}"
    );
    assert!(!dir.path().join("forged_bomb").exists());

    // 8c. Zip directory entry marked via Unix S_IFDIR mode without trailing `/` extracts as dir cleanly
    let s_ifdir_zip = dir.path().join("unix_dir.zip");
    {
        let mut zw = ZipWriter::new(File::create(&s_ifdir_zip).unwrap());
        zw.start_file(
            "subdir",
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o755),
        )
        .unwrap();
        zw.start_file("subdir/hello.txt", SimpleFileOptions::default())
            .unwrap();
        zw.write_all(b"inside unix S_IFDIR").unwrap();
        zw.finish().unwrap();
    }
    {
        let mut raw = fs::read(&s_ifdir_zip).unwrap();
        let mut patched = false;
        for idx in 0..raw.len().saturating_sub(52) {
            if &raw[idx..idx + 4] == b"PK\x01\x02" {
                let nlen = u16::from_le_bytes([raw[idx + 28], raw[idx + 29]]) as usize;
                if nlen == 6 && idx + 46 + nlen <= raw.len() && &raw[idx + 46..idx + 52] == b"subdir" {
                    let dir_mode = (0o040755u16).to_le_bytes();
                    raw[idx + 40..idx + 42].copy_from_slice(&dir_mode);
                    patched = true;
                }
            }
        }
        assert!(patched, "failed to locate central header for subdir");
        fs::write(&s_ifdir_zip, &raw).unwrap();
    }
    let s_ifdir_res =
        convert_files(std::slice::from_ref(&s_ifdir_zip), &["UNZIP".into()], &prefs, &cancel, None)
            .unwrap();
    assert!(s_ifdir_res[0].error.is_none(), "{:?}", s_ifdir_res[0].error);
    let s_ifdir_out = PathBuf::from(&s_ifdir_res[0].outputs[0]);
    assert!(s_ifdir_out.join("subdir").is_dir());
    assert_eq!(
        fs::read_to_string(s_ifdir_out.join("subdir/hello.txt")).unwrap(),
        "inside unix S_IFDIR"
    );

    assert_no_residual_staging(dir.path());
}

#[test]
fn vision_remove_bg_cuts_subject_and_rejects_blank_image() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    let src_cover = repo_root().join("docs/assets/flip-cover.png");
    let cover_png = dir.path().join("cover.png");
    fs::copy(&src_cover, &cover_png).unwrap();
    let cover_hash = sha256_file(&cover_png);

    let bg_res = convert_files(
        std::slice::from_ref(&cover_png),
        &["REMOVE_BG".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(bg_res[0].error.is_none(), "REMOVE_BG failed: {:?}", bg_res[0].error);
    let out_png = PathBuf::from(&bg_res[0].outputs[0]);
    assert_eq!(out_png.file_name().unwrap().to_str().unwrap(), "cover 2.png");
    assert_eq!(sha256_file(&cover_png), cover_hash);

    let rgba = image::open(&out_png).unwrap().to_rgba8();
    assert_eq!(rgba.dimensions(), image::image_dimensions(&cover_png).unwrap());
    let transparent_px = rgba.pixels().filter(|p| p[3] < 20).count();
    let opaque_px = rgba.pixels().filter(|p| p[3] > 200).count();
    assert!(transparent_px > 1000, "expected background transparency, got {transparent_px}");
    assert!(opaque_px > 1000, "expected foreground subject pixels, got {opaque_px}");

    // Solid uniform frame has no foreground subject and must fail cleanly without creating output
    let blank_png = dir.path().join("blank.png");
    let blank_img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(120, 90, Rgba([128, 128, 128, 255]));
    blank_img.save_with_format(&blank_png, ImageFormat::Png).unwrap();

    let blank_res = convert_files(
        std::slice::from_ref(&blank_png),
        &["REMOVE_BG".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    let blank_err = blank_res[0].error.as_deref().unwrap_or("");
    assert!(
        !blank_err.is_empty()
            && (blank_err.to_ascii_lowercase().contains("foreground")
                || blank_err.to_ascii_lowercase().contains("subject")),
        "unexpected blank REMOVE_BG error: {blank_err:?}"
    );
    assert!(!dir.path().join("blank 2.png").exists());

    // EXIF Orientation=6 JPEG normalizes upright before Vision mask generation
    let exif6_jpg = dir.path().join("cover_exif6.jpg");
    let mk_exif6 = dir.path().join("mk_exif6_bg.swift");
    fs::write(
        &mk_exif6,
        format!(
            r#"import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, nil)!
let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let w = cg.width, h = cg.height
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let rot = CGContext(data: nil, width: h, height: w, bitsPerComponent: 8, bytesPerRow: 0, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
rot.translateBy(x: CGFloat(h), y: 0)
rot.rotate(by: .pi / 2)
rot.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
let rotCg = rot.makeImage()!
let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: "{}") as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(dest, rotCg, [kCGImagePropertyOrientation: 6] as CFDictionary)
CGImageDestinationFinalize(dest)
"#,
            cover_png.display(),
            exif6_jpg.display()
        ),
    )
    .unwrap();
    assert!(Command::new("/usr/bin/swift").arg(&mk_exif6).status().unwrap().success());
    let exif6_res = convert_files(
        std::slice::from_ref(&exif6_jpg),
        &["REMOVE_BG".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(exif6_res[0].error.is_none(), "EXIF6 REMOVE_BG failed: {:?}", exif6_res[0].error);
    let exif6_out = PathBuf::from(&exif6_res[0].outputs[0]);
    let exif6_rgba = image::open(&exif6_out).unwrap().to_rgba8();
    assert_eq!(
        exif6_rgba.dimensions(),
        image::image_dimensions(&cover_png).unwrap(),
        "EXIF Orientation=6 REMOVE_BG output must be upright"
    );
    assert!(exif6_rgba.pixels().filter(|p| p[3] < 20).count() > 1000);

    assert_no_residual_staging(dir.path());
}

#[test]
fn optimize_pdf_shrinks_heavy_pdf_preserves_text_and_rejects_over_200_pages() {
    let dir = tempdir().unwrap();
    let prefs = AppPreferences::default();
    let cancel = Arc::new(AtomicBool::new(false));

    // 1. 2-page PDF with heavy uncompressed RGB images + selectable Helvetica text
    let heavy_pdf = dir.path().join("heavy.pdf");
    build_test_pdf(&heavy_pdf, 2, true);
    let heavy_len = fs::metadata(&heavy_pdf).unwrap().len();
    let heavy_hash = sha256_file(&heavy_pdf);
    assert!(heavy_len > 500_000, "expected uncompressed heavy PDF, got {heavy_len}");

    let opt_res = convert_files(
        std::slice::from_ref(&heavy_pdf),
        &["OPTIMIZE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(opt_res[0].error.is_none(), "OPTIMIZE_PDF failed: {:?}", opt_res[0].error);
    let out_pdf = PathBuf::from(&opt_res[0].outputs[0]);
    assert_eq!(out_pdf.file_name().unwrap().to_str().unwrap(), "heavy 2.pdf");
    assert_eq!(sha256_file(&heavy_pdf), heavy_hash);

    let out_bytes = fs::read(&out_pdf).unwrap();
    assert!(out_bytes.starts_with(b"%PDF-"));
    assert!(out_bytes.windows(5).any(|w| w == b"%%EOF"));
    assert!(
        (out_bytes.len() as u64) < heavy_len / 2,
        "expected Quartz OPTIMIZE_PDF size reduction (< {}), got {}",
        heavy_len / 2,
        out_bytes.len()
    );

    // Selectable vector text must still extract cleanly after optimization
    let txt_res = convert_files(
        std::slice::from_ref(&out_pdf),
        &["TXT".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(txt_res[0].error.is_none(), "TXT extract failed: {:?}", txt_res[0].error);
    let extracted = fs::read_to_string(&txt_res[0].outputs[0]).unwrap();
    assert!(
        extracted.contains("FLIP Selectable Marker Page 1")
            && extracted.contains("FLIP Selectable Marker Page 2"),
        "Optimized PDF lost selectable text:\n{extracted}"
    );

    // 2. Reject 201-page PDF cleanly with 200-page safety limit
    let pdf_201 = dir.path().join("too_many_pages.pdf");
    build_test_pdf(&pdf_201, 201, false);
    let over_res = convert_files(
        std::slice::from_ref(&pdf_201),
        &["OPTIMIZE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    let over_err = over_res[0].error.as_deref().unwrap_or("");
    assert!(
        over_err.contains("200"),
        "expected 200-page limit error, got: {over_err:?}"
    );
    assert!(!dir.path().join("too_many_pages 2.pdf").exists());

    // 3. Reject locked/encrypted PDF and preserve CropBox + Rotate 90 dimensions
    let locked_pdf = dir.path().join("locked_opt.pdf");
    let crop_rot_pdf = dir.path().join("crop_rot90.pdf");
    let crop_raw = b"%PDF-1.4\n\
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /CropBox [50 50 350 450] /Rotate 90 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj\n\
4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
5 0 obj << /Length 44 >> stream\n\
BT /F1 18 Tf 100 200 Td (CROP ROT90) Tj ET\n\
endstream endobj\n\
xref\n0 6\n\
0000000000 65535 f \n\
0000000009 00000 n \n\
0000000058 00000 n \n\
0000000115 00000 n \n\
0000000284 00000 n \n\
0000000352 00000 n \n\
trailer << /Size 6 /Root 1 0 R >>\nstartxref\n446\n%%EOF\n";
    fs::write(&crop_rot_pdf, crop_raw).unwrap();

    let lock_script = dir.path().join("lock_opt_pdf.swift");
    fs::write(
        &lock_script,
        format!(
            r#"import Foundation
import PDFKit
let doc = PDFDocument(url: URL(fileURLWithPath: "{}"))!
doc.write(to: URL(fileURLWithPath: "{}"), withOptions: [.userPasswordOption: "secret", .ownerPasswordOption: "owner"])
"#,
            heavy_pdf.display(),
            locked_pdf.display()
        ),
    )
    .unwrap();
    assert!(Command::new("/usr/bin/swift").arg(&lock_script).status().unwrap().success());

    let locked_res = convert_files(
        std::slice::from_ref(&locked_pdf),
        &["OPTIMIZE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(
        locked_res[0]
            .error
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("password"),
        "expected password/encrypted error, got: {:?}",
        locked_res[0].error
    );

    let crop_opt_res = convert_files(
        std::slice::from_ref(&crop_rot_pdf),
        &["OPTIMIZE_PDF".into()],
        &prefs,
        &cancel,
        None,
    )
    .unwrap();
    assert!(crop_opt_res[0].error.is_none(), "{:?}", crop_opt_res[0].error);
    let crop_opt_out = PathBuf::from(&crop_opt_res[0].outputs[0]);
    let check_crop = dir.path().join("check_crop_opt.swift");
    fs::write(
        &check_crop,
        format!(
            r#"import Foundation
import PDFKit
let doc = PDFDocument(url: URL(fileURLWithPath: "{}"))!
let page = doc.page(at: 0)!
let box = page.bounds(for: .cropBox)
// CropBox [50 50 350 450] is 300x400; rotated 90 degrees becomes 400x300
assert(abs(box.width - 400.0) < 2.0 && abs(box.height - 300.0) < 2.0, "unexpected cropped rotated bounds: \(box)")
assert((page.string ?? "").contains("CROP ROT90"), "lost text: \(page.string ?? "")")
print("CROP_ROT_OK")
"#,
            crop_opt_out.display()
        ),
    )
    .unwrap();
    let check_out = Command::new("/usr/bin/swift").arg(&check_crop).output().unwrap();
    assert!(
        check_out.status.success() && String::from_utf8_lossy(&check_out.stdout).contains("CROP_ROT_OK"),
        "CropBox/Rotate90 check failed: {}",
        String::from_utf8_lossy(&check_out.stderr)
    );

    assert_no_residual_staging(dir.path());
}
