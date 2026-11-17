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

    let locked_png = convert_files(&[locked_pdf.clone()], &["PNG".into()], &prefs, &cancel, None).unwrap();
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
    let jpg_nogps = convert_files(&[exif_src.clone()], &["TIFF".into()], &no_gps_prefs, &cancel, None).unwrap();
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

        let mov_res = convert_files(&[odd_mp4.clone()], &["MOV".into()], &prefs, &cancel, None).unwrap();
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
    bridge::convert_pdf_from_images(&blank_pdf, &[blank_img.clone()], None).unwrap();
    let blank_pdf_txt = dir.path().join("blank_pdf.txt");
    let blank_pdf_err = bridge::convert_pdf_extract_text(&blank_pdf, &blank_pdf_txt, None).unwrap_err();
    assert!(blank_pdf_err.to_string().contains("OCR"));
    assert!(!blank_pdf_txt.exists(), "0-byte TXT must NOT be written on empty PDF OCR");

    // 4. imagesToPDF A4 595x842 layout, >128 guard, and no-overwrite guard
    let combined_a4 = dir.path().join("combined_a4.pdf");
    bridge::convert_pdf_from_images(&combined_a4, &[rotated_ocr_jpg, blank_img.clone()], None).unwrap();
    // Overwrite refusal
    assert!(bridge::convert_pdf_from_images(&combined_a4, &[blank_img.clone()], None).is_err());
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
