use crate::core::logging::log_event;
use crate::core::naming::{clipboard_base_stem, next_available_new_item_path};
use crate::core::preferences::AppPreferences;
use crate::engines::{bridge, EngineError};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFileKindInfo {
    pub id: String,
    pub name: String,
    pub extension: Option<String>,
    pub category: String,
    pub page: u8,
    pub available: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTemplateInfo {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
    pub extension: Option<String>,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNewResult {
    pub path: String,
    pub kind_id: String,
    pub revealed: bool,
    pub rename_triggered: bool,
    pub fallback_used: bool,
}

pub fn built_in_kinds(clipboard_available: bool) -> Vec<NewFileKindInfo> {
    let has_pages = Path::new("/Applications/Pages.app").exists();
    let has_numbers = Path::new("/Applications/Numbers.app").exists();
    let has_keynote = Path::new("/Applications/Keynote.app").exists();

    vec![
        NewFileKindInfo {
            id: "folder".into(),
            name: "Folder".into(),
            extension: None,
            category: "system".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "txt".into(),
            name: "Plain Text".into(),
            extension: Some("txt".into()),
            category: "document".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "md".into(),
            name: "Markdown".into(),
            extension: Some("md".into()),
            category: "document".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "rtf".into(),
            name: "Rich Text".into(),
            extension: Some("rtf".into()),
            category: "document".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "docx".into(),
            name: "Word Document".into(),
            extension: Some("docx".into()),
            category: "office".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "xlsx".into(),
            name: "Spreadsheet".into(),
            extension: Some("xlsx".into()),
            category: "office".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "html".into(),
            name: "HTML".into(),
            extension: Some("html".into()),
            category: "code".into(),
            page: 1,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "clipboard".into(),
            name: "From Clipboard".into(),
            extension: None,
            category: "clipboard".into(),
            page: 1,
            available: clipboard_available,
            disabled_reason: if clipboard_available {
                None
            } else {
                Some("Clipboard does not contain image, PDF, RTF, or text".into())
            },
        },
        NewFileKindInfo {
            id: "json".into(),
            name: "JSON".into(),
            extension: Some("json".into()),
            category: "data".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "csv".into(),
            name: "CSV".into(),
            extension: Some("csv".into()),
            category: "data".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "yaml".into(),
            name: "YAML Config".into(),
            extension: Some("yaml".into()),
            category: "data".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "toml".into(),
            name: "TOML Config".into(),
            extension: Some("toml".into()),
            category: "data".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "xml".into(),
            name: "XML Document".into(),
            extension: Some("xml".into()),
            category: "data".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "pdf".into(),
            name: "Blank PDF (A4)".into(),
            extension: Some("pdf".into()),
            category: "document".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "sh".into(),
            name: "Shell Script".into(),
            extension: Some("sh".into()),
            category: "code".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "py".into(),
            name: "Python".into(),
            extension: Some("py".into()),
            category: "code".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "swift".into(),
            name: "Swift".into(),
            extension: Some("swift".into()),
            category: "code".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "pptx".into(),
            name: "Presentation".into(),
            extension: Some("pptx".into()),
            category: "office".into(),
            page: 2,
            available: true,
            disabled_reason: None,
        },
        NewFileKindInfo {
            id: "pages".into(),
            name: "Pages Document".into(),
            extension: Some("pages".into()),
            category: "iwork".into(),
            page: 2,
            available: has_pages,
            disabled_reason: if has_pages { None } else { Some("Pages.app not installed".into()) },
        },
        NewFileKindInfo {
            id: "numbers".into(),
            name: "Numbers Sheet".into(),
            extension: Some("numbers".into()),
            category: "iwork".into(),
            page: 2,
            available: has_numbers,
            disabled_reason: if has_numbers { None } else { Some("Numbers.app not installed".into()) },
        },
        NewFileKindInfo {
            id: "keynote".into(),
            name: "Keynote Deck".into(),
            extension: Some("key".into()),
            category: "iwork".into(),
            page: 2,
            available: has_keynote,
            disabled_reason: if has_keynote { None } else { Some("Keynote.app not installed".into()) },
        },
    ]
}

pub fn ensure_templates_dir_initialized(templates_dir: &Path) -> Result<(), EngineError> {
    if !templates_dir.exists() {
        fs::create_dir_all(templates_dir)?;
    }
    let readme = templates_dir.join("README.txt");
    if fs::symlink_metadata(&readme).is_err() {
        let body = "FLIP User Templates\n\
            ===================\n\n\
            Drop any file into this folder to add it to FLIP's New File wheel\n\
            and the Menu Bar 'New File in Finder Folder' submenu.\n\n\
            Organization:\n\
              - Files in this root folder appear directly under Custom Templates.\n\
              - Subfolders become template groups (e.g. Notes/Meeting Note.md).\n\
              - Executable scripts (.sh or +x) preserve executable permissions.\n\n\
            Supported placeholders inside plain-text templates (.txt, .md, .html, .json, .sh, .py, .swift, etc.):\n\
              {{date}}   - Current local date (YYYY-MM-DD)\n\
              {{time}}   - Current local time (HH:MM)\n\
              {{user}}   - Current macOS username\n\
              {{folder}} - Active Finder destination folder name\n";
        fs::write(&readme, body)?;
    }
    Ok(())
}

pub fn scan_user_templates(templates_dir: &Path) -> Vec<UserTemplateInfo> {
    let mut items = Vec::new();
    if !templates_dir.is_dir() {
        return items;
    }
    collect_templates_recursive(templates_dir, None, &mut items);
    items.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
    items
}

fn collect_templates_recursive(
    dir: &Path,
    group: Option<&str>,
    out: &mut Vec<UserTemplateInfo>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name_os = entry.file_name();
        let fname = name_os.to_string_lossy();
        if fname.starts_with('.') || (group.is_none() && fname == "README.txt") {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let dir_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(
                dir_ext.as_str(),
                "pages"
                    | "numbers"
                    | "key"
                    | "rtfd"
                    | "app"
                    | "bundle"
                    | "framework"
                    | "xcodeproj"
                    | "xcworkspace"
                    | "playground"
            ) {
                continue;
            }
            let next_group = match group {
                Some(g) => format!("{g}/{fname}"),
                None => fname.into_owned(),
            };
            collect_templates_recursive(&path, Some(&next_group), out);
        } else if meta.is_file() {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&fname)
                .to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string());
            let id = format!(
                "template:{}",
                path.to_string_lossy()
            );
            out.push(UserTemplateInfo {
                id,
                name: stem,
                group: group.map(String::from),
                extension: ext,
                source_path: path.to_string_lossy().into_owned(),
            });
        }
    }
}

pub fn expand_placeholders(raw: &str, target_dir: &Path) -> String {
    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H:%M").to_string();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "user".to_string());
    let folder = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Desktop")
        .to_string();

    raw.replace("{{date}}", &date_str)
        .replace("{{time}}", &time_str)
        .replace("{{user}}", &user)
        .replace("{{folder}}", &folder)
}

fn verify_writable_directory(dir: &Path) -> Result<(), EngineError> {
    if !dir.is_dir() {
        return Err(EngineError::ConversionFailed(format!(
            "Can't create files here: `{}` is not an accessible directory",
            dir.display()
        )));
    }
    let meta = fs::metadata(dir)?;
    if meta.permissions().readonly() {
        return Err(EngineError::ConversionFailed(format!(
            "Can't create files here: `{}` is read-only",
            dir.display()
        )));
    }
    Ok(())
}

pub fn resolve_target_folder(
    explicit_dir: Option<&Path>,
    prefs: &AppPreferences,
) -> (PathBuf, bool) {
    if let Some(dir) = explicit_dir {
        if dir.is_dir() {
            return (dir.to_path_buf(), false);
        }
    }
    if let Ok(ctx) = bridge::finder_context() {
        let p = PathBuf::from(&ctx.target_folder);
        if p.is_dir() {
            return (p, false);
        }
    }
    (prefs.resolved_fallback_folder(), true)
}

pub fn create_new_item(
    kind_id: &str,
    explicit_dir: Option<&Path>,
    prefs: &AppPreferences,
    reveal: bool,
) -> Result<CreateNewResult, EngineError> {
    let (target_dir, fallback_used) = resolve_target_folder(explicit_dir, prefs);
    verify_writable_directory(&target_dir)?;

    let created_path: PathBuf;

    if let Some(tmpl_path_str) = kind_id.strip_prefix("template:") {
        let tmpl_path = Path::new(tmpl_path_str);
        let meta = fs::symlink_metadata(tmpl_path).map_err(|_| {
            EngineError::ConversionFailed(format!("Template file not found: {tmpl_path_str}"))
        })?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(EngineError::ConversionFailed(format!(
                "Template must be a regular non-symlink file: {tmpl_path_str}"
            )));
        }
        let ext = tmpl_path.extension().and_then(|e| e.to_str());
        let stem = tmpl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let dest = next_available_new_item_path(&target_dir, stem, ext)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

        let ext_low = ext.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let is_text = matches!(
            ext_low.as_str(),
            "txt"
                | "md"
                | "markdown"
                | "html"
                | "htm"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "csv"
                | "sh"
                | "zsh"
                | "bash"
                | "py"
                | "swift"
                | "rs"
                | "ts"
                | "js"
                | "css"
                | "sql"
        );
        if is_text {
            let raw = fs::read_to_string(tmpl_path)?;
            let expanded = expand_placeholders(&raw, &target_dir);
            write_new_file(&dest, expanded.as_bytes())?;
        } else {
            let bytes = fs::read(tmpl_path)?;
            write_new_file(&dest, &bytes)?;
        }
        // Preserve executable bit if source was executable or shell script
        if let Ok(src_meta) = fs::metadata(tmpl_path) {
            let mode = src_meta.permissions().mode();
            if (mode & 0o111) != 0 || ext_low == "sh" {
                let mut p = fs::metadata(&dest)?.permissions();
                p.set_mode(0o755);
                fs::set_permissions(&dest, p)?;
            }
        }
        created_path = dest;
    } else {
        created_path = match kind_id {
            "folder" => {
                let dest = next_available_new_item_path(&target_dir, "untitled folder", None)
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                fs::create_dir(&dest)?;
                dest
            }
            "txt" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("txt"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"")?;
                dest
            }
            "md" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("md"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"# Untitled\n\n")?;
                dest
            }
            "rtf" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("rtf"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, blank_rtf_bytes())?;
                dest
            }
            "docx" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("docx"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, &blank_docx_bytes()?)?;
                dest
            }
            "xlsx" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("xlsx"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, &blank_xlsx_bytes()?)?;
                dest
            }
            "pptx" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("pptx"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, &blank_pptx_bytes()?)?;
                dest
            }
            "html" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("html"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, blank_html_bytes())?;
                dest
            }
            "json" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("json"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"{\n  \n}\n")?;
                dest
            }
            "csv" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("csv"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"column1,column2\n")?;
                dest
            }
            "yaml" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("yaml"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"# Untitled\nname: untitled\n")?;
                dest
            }
            "toml" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("toml"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"# Untitled\ntitle = \"Untitled\"\n")?;
                dest
            }
            "xml" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("xml"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(
                    &dest,
                    b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root>\n  \n</root>\n",
                )?;
                dest
            }
            "pdf" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("pdf"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, &blank_pdf_bytes())?;
                dest
            }
            "sh" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("sh"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"#!/bin/zsh\nset -euo pipefail\n\n")?;
                let mut perms = fs::metadata(&dest)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&dest, perms)?;
                dest
            }
            "py" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("py"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(
                    &dest,
                    b"#!/usr/bin/env python3\n\ndef main() -> None:\n    pass\n\nif __name__ == \"__main__\":\n    main()\n",
                )?;
                dest
            }
            "swift" => {
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("swift"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                write_new_file(&dest, b"import Foundation\n\n")?;
                dest
            }
            "pages" => {
                if !Path::new("/Applications/Pages.app").exists() {
                    return Err(EngineError::Unsupported("Pages.app is not installed".to_string()));
                }
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("pages"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                bridge::create_iwork("pages", &dest)?;
                dest
            }
            "numbers" => {
                if !Path::new("/Applications/Numbers.app").exists() {
                    return Err(EngineError::Unsupported("Numbers.app is not installed".to_string()));
                }
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("numbers"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                bridge::create_iwork("numbers", &dest)?;
                dest
            }
            "keynote" => {
                if !Path::new("/Applications/Keynote.app").exists() {
                    return Err(EngineError::Unsupported("Keynote.app is not installed".to_string()));
                }
                let dest = next_available_new_item_path(&target_dir, "Untitled", Some("key"))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                bridge::create_iwork("key", &dest)?;
                dest
            }
            "clipboard" => {
                let clip = bridge::clipboard_inspect()?;
                if !clip.available || clip.extension.is_empty() {
                    return Err(EngineError::Unsupported(
                        "Clipboard does not contain supported image, PDF, RTF, or text data"
                            .to_string(),
                    ));
                }
                let stem = clipboard_base_stem();
                let dest =
                    next_available_new_item_path(&target_dir, &stem, Some(&clip.extension))
                        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                let save_res = bridge::clipboard_save(&dest)?;
                if save_res.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    return Err(EngineError::ConversionFailed(
                        "Failed to write clipboard contents".to_string(),
                    ));
                }
                dest
            }
            other => {
                return Err(EngineError::Unsupported(format!(
                    "Unknown New File kind: `{other}`"
                )))
            }
        };
    }

    let mut revealed = false;
    let mut rename_triggered = false;
    if reveal {
        if let Ok(r) = bridge::finder_reveal(&[created_path.as_path()], prefs.auto_rename_on_create) {
            revealed = r.ok;
            rename_triggered = r.rename_triggered;
        }
    }

    log_event(
        "create_new.completed",
        Some(json!({
            "kind": kind_id,
            "path": created_path.to_string_lossy(),
            "revealed": revealed,
            "renameTriggered": rename_triggered,
        })),
    );

    Ok(CreateNewResult {
        path: created_path.to_string_lossy().into_owned(),
        kind_id: kind_id.to_string(),
        revealed,
        rename_triggered,
        fallback_used,
    })
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), EngineError> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn blank_rtf_bytes() -> &'static [u8] {
    br"{\rtf1\ansi\ansicpg1252\cocoartf2761
{\fonttbl\f0\fswiss\fcharset0 Helvetica;}
{\colortbl;\red255\green255\blue255;}
\margl1440\margr1440\vieww11520\viewh8400\viewkind0
\pard\tx720\tx1440\tx2160\tx2880\tx3600\tx4320\tx5040\tx5760\tx6480\tx7200\tx7920\tx8640\pardirnatural\partightenfactor0
\f0\fs24 \cf0 }"
}

pub fn blank_html_bytes() -> &'static [u8] {
    b"<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\" />\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n  <title>Untitled</title>\n</head>\n<body>\n  \n</body>\n</html>\n"
}

pub fn blank_pdf_bytes() -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(4);
    let objects: [&[u8]; 4] = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> /Contents 4 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Length 0 >>\nstream\n\nendstream\nendobj\n",
    ];
    for obj in objects {
        offsets.push(buf.len());
        buf.extend_from_slice(obj);
    }
    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for off in offsets {
        buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
            .as_bytes(),
    );
    buf
}

fn write_zip_entries(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, EngineError> {
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in entries {
        zip.start_file(*name, opts)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        zip.write_all(content)?;
    }
    let finished = zip
        .finish()
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    Ok(finished.into_inner())
}

pub fn blank_docx_bytes() -> Result<Vec<u8>, EngineError> {
    let content_types = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
    let rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
    let document_xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t></w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/>
    </w:sectPr>
  </w:body>
</w:document>"#;

    write_zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("word/document.xml", document_xml),
    ])
}

pub fn blank_xlsx_bytes() -> Result<Vec<u8>, EngineError> {
    let content_types = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;
    let rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
    let wb_rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;
    let workbook = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#;
    let sheet1 = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData/>
</worksheet>"#;
    let styles = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;

    write_zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/workbook.xml", workbook),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/styles.xml", styles),
    ])
}

pub fn blank_pptx_bytes() -> Result<Vec<u8>, EngineError> {
    let content_types = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
</Types>"#;
    let rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#;
    let pres_rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;
    let presentation = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
  <p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>"#;
    let slide1_rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;
    let slide1 = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>
</p:sld>"#;
    let layout_rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#;
    let layout1 = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank">
  <p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>
</p:sldLayout>"#;
    let master_rels = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#;
    let master1 = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>
  <p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>
</p:sldMaster>"#;

    write_zip_entries(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("ppt/_rels/presentation.xml.rels", pres_rels),
        ("ppt/presentation.xml", presentation),
        ("ppt/slides/_rels/slide1.xml.rels", slide1_rels),
        ("ppt/slides/slide1.xml", slide1),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", layout_rels),
        ("ppt/slideLayouts/slideLayout1.xml", layout1),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", master_rels),
        ("ppt/slideMasters/slideMaster1.xml", master1),
    ])
}
