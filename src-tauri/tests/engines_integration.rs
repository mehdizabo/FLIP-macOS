use flip_lib::core::policy::EngineCapabilities;
use flip_lib::core::preferences::AppPreferences;
use flip_lib::engines::create_new::{
    built_in_kinds, create_new_item, ensure_templates_dir_initialized, scan_user_templates,
};
use flip_lib::engines::{bridge, convert_files, detect_tools, media, EngineError};
use image::{GenericImageView, ImageBuffer, ImageFormat, Rgba};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
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

    let res = convert_files(&[png.clone()], &formats, &prefs, &cancel, None).unwrap();
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
        &[webp.clone()],
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

    let rot_res = convert_files(&[rotated.clone()], &["PNG".into()], &prefs, &cancel, None).unwrap();
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
    let res = convert_files(&[json_path.clone()], &targets, &prefs, &cancel, None).unwrap();
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
    let csv_to_json = convert_files(&[csv_out.clone()], &["JSON".into()], &prefs, &cancel, None).unwrap();
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

    let mut doc_targets: Vec<String> = ["PDF", "DOCX", "MD", "HTML", "RTF", "ODT", "PNG", "JPG", "WEBP"]
        .into_iter()
        .map(String::from)
        .collect();
    if tools.pandoc.is_some() {
        doc_targets.extend(["EPUB", "FB2", "RST"].into_iter().map(String::from));
    }

    let doc_res = convert_files(&[txt.clone()], &doc_targets, &prefs, &cancel, None).unwrap();
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
        &[pdf_path.clone()],
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
    let ocr_pdf_res = convert_files(&[scanned_text_pdf.clone()], &["TXT".into()], &prefs, &cancel, None).unwrap();
    assert!(ocr_pdf_res[0].error.is_none(), "OCR PDF failed: {:?}", ocr_pdf_res[0].error);
    let ocr_text = fs::read_to_string(&ocr_pdf_res[0].outputs[0]).unwrap();
    assert!(ocr_text.contains("FLIP"), "OCR PDF missed FLIP: {ocr_text}");

    // Blank shape PDF (no text anywhere) -> TXT rejects with OCR error rather than empty file
    let blank_img = dir.path().join("shape.png");
    make_transparent_test_png(&blank_img);
    let blank_pdf = dir.path().join("shape.pdf");
    bridge::convert_pdf_from_images(&blank_pdf, &[blank_img], None).unwrap();
    let blank_res = convert_files(&[blank_pdf.clone()], &["TXT".into()], &prefs, &cancel, None).unwrap();
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
            &[tick_wav.clone()],
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
            &[mp4.clone()],
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
    let mut prefs = AppPreferences::default();
    prefs.output_subfolder = Some("Converted".to_string());

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
