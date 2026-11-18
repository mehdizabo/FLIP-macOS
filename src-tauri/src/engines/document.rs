use crate::core::naming::format_extension;
use crate::core::policy::{
    ext_lower, PolicyError, MAX_CONTAINER_ENTRIES, MAX_CONTAINER_EXPANDED_BYTES,
};
use crate::core::preferences::AppPreferences;
use crate::engines::{bridge, pdf, DetectedTools, EngineError};
use chrono::Utc;
use pulldown_cmark::{html, Options, Parser};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const MAX_TEXT_READ_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_HTML_READ_BYTES: u64 = 20 * 1024 * 1024;
const MAX_EPUB_TEXT_READ_BYTES: u64 = 32 * 1024 * 1024;

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn plain_text_to_html(title: &str, text: &str) -> String {
    let mut paragraphs = String::new();
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if line.is_empty() {
            paragraphs.push_str("<p>&nbsp;</p>\n");
        } else {
            paragraphs.push_str(&format!("<p>{}</p>\n", html_escape(line)));
        }
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:11.5pt;line-height:1.45;color:#111;}}p{{margin:0 0 6pt 0;}}</style>\
        </head><body>\n{}</body></html>",
        html_escape(title),
        paragraphs
    )
}

pub fn markdown_to_html(title: &str, md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut body = String::new();
    html::push_html(&mut body, parser);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:11.5pt;line-height:1.5;color:#111;}}h1,h2,h3{{margin:14pt 0 6pt 0;}}pre,code{{font-family:Menlo,monospace;font-size:10pt;background:#f4f4f6;}}table{{border-collapse:collapse;width:100%;}}th,td{{border:1px solid #ccc;padding:4pt 6pt;}}</style>\
        </head><body>\n{body}</body></html>",
        html_escape(title)
    )
}

fn decode_named_html_entity(entity: &str) -> Option<&'static str> {
    match entity {
        "nbsp" => Some(" "),
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "quot" => Some("\""),
        "apos" => Some("'"),
        "mdash" => Some("—"),
        "ndash" => Some("–"),
        "lsquo" => Some("‘"),
        "rsquo" => Some("’"),
        "ldquo" => Some("“"),
        "rdquo" => Some("”"),
        "hellip" => Some("…"),
        "bull" => Some("•"),
        "copy" => Some("©"),
        "reg" => Some("®"),
        "trade" => Some("™"),
        "euro" => Some("€"),
        "deg" => Some("°"),
        "times" => Some("×"),
        "divide" => Some("÷"),
        "laquo" => Some("«"),
        "raquo" => Some("»"),
        "sect" => Some("§"),
        "para" => Some("¶"),
        _ => None,
    }
}

fn decode_html_entities_inner(s: &str, nbsp_as_unicode: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi_rel) = s[i..].find(';') {
                if semi_rel <= 12 {
                    let entity = &s[i + 1..i + semi_rel];
                    let decoded = if entity == "nbsp" {
                        Some(if nbsp_as_unicode { "\u{00A0}" } else { " " }.to_string())
                    } else if let Some(named) = decode_named_html_entity(entity) {
                        Some(named.to_string())
                    } else if entity.starts_with("#x") || entity.starts_with("#X") {
                        u32::from_str_radix(&entity[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string())
                    } else if let Some(rest) = entity.strip_prefix('#') {
                        rest.parse::<u32>()
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string())
                    } else {
                        None
                    };
                    if let Some(ch_str) = decoded {
                        out.push_str(&ch_str);
                        i += semi_rel + 1;
                        continue;
                    }
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_html_entities(s: &str) -> String {
    decode_html_entities_inner(s, false)
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}') {
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn xml_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}') {
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn extract_attr(tag_inner: &str, attr_name: &str) -> Option<String> {
    parse_html_attrs(tag_inner)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(attr_name))
        .map(|(_, v)| v)
}

fn parse_html_attrs(tag_inner: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = tag_inner.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Skip tag name
    while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
        i += 1;
    }

    while i < len {
        while i < len && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= len {
            break;
        }
        let name_start = i;
        while i < len
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
            && bytes[i] != b'>'
        {
            i += 1;
        }
        let name = tag_inner[name_start..i].trim();
        if name.is_empty() {
            i += 1;
            continue;
        }
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < len && bytes[i] == b'=' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let v_start = i;
                while i < len && bytes[i] != q {
                    i += 1;
                }
                let v = &tag_inner[v_start..i];
                if i < len {
                    i += 1;
                }
                v.to_string()
            } else {
                let v_start = i;
                while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                tag_inner[v_start..i]
                    .trim_end_matches('/')
                    .to_string()
            }
        } else {
            name.to_ascii_lowercase()
        };
        attrs.push((name.to_ascii_lowercase(), value));
    }
    attrs
}

fn has_xml_self_closing_slash(inner: &str) -> bool {
    let trimmed = inner.trim_end();
    if !trimmed.ends_with('/') {
        return false;
    }
    let before = trimmed
        .as_bytes()
        .get(trimmed.len().saturating_sub(2))
        .copied();
    matches!(before, Some(b) if b.is_ascii_whitespace() || b == b'"' || b == b'\'')
        || !trimmed.contains('=')
}

fn find_raw_close_tag(input: &str, from: usize, tag_name: &str) -> Option<usize> {
    let lower = input[from..].to_ascii_lowercase();
    let needle = format!("</{tag_name}");
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(&needle) {
        let start = search_from + rel;
        let after = start + needle.len();
        let next_b = lower.as_bytes().get(after).copied();
        if matches!(next_b, Some(b) if b == b'>' || b.is_ascii_whitespace()) {
            if let Some(gt_rel) = lower[after..].find('>') {
                return Some(from + after + gt_rel + 1);
            }
            return None;
        }
        search_from = after;
    }
    None
}

pub fn html_to_markdown(html_input: &str) -> String {
    let mut out = String::with_capacity(html_input.len());
    let mut skip_tag: Option<String> = None;
    let mut in_pre = false;
    let mut list_stack: Vec<(bool, usize)> = Vec::new(); // (is_ordered, count)
    let mut link_hrefs: Vec<Option<String>> = Vec::new();
    let mut table_row_cells: Vec<String> = Vec::new();
    let mut in_table_row = false;
    let mut table_has_header_sep = false;
    let mut current_cell = String::new();
    let mut in_cell = false;

    let bytes = html_input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if html_input[i..].starts_with("<!--") {
                if let Some(end_rel) = html_input[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                } else {
                    break;
                }
                continue;
            }
            if html_input[i..].starts_with("<![CDATA[") {
                let (cdata_end, next_i) = if let Some(end_rel) = html_input[i + 9..].find("]]>") {
                    (i + 9 + end_rel, i + 9 + end_rel + 3)
                } else {
                    (bytes.len(), bytes.len())
                };
                if skip_tag.is_none() {
                    let target = if in_cell { &mut current_cell } else { &mut out };
                    target.push_str(&html_input[i + 9..cdata_end]);
                }
                i = next_i;
                continue;
            }
            if !matches!(
                bytes.get(i + 1),
                Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')
            ) {
                if skip_tag.is_none() {
                    let target = if in_cell { &mut current_cell } else { &mut out };
                    target.push('<');
                }
                i += 1;
                continue;
            }

            // Find end of tag respecting quotes
            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            while j < bytes.len() {
                let b = bytes[j];
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'>' {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                if skip_tag.is_none() {
                    out.push_str(&html_input[i..]);
                }
                break;
            }

            let raw_tag = html_input[i + 1..j].trim();
            i = j + 1;

            if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }

            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let self_closing_syntax = !is_close && has_xml_self_closing_slash(inner);
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if !is_close && matches!(tag_name.as_str(), "script" | "style") && !self_closing_syntax {
                i = find_raw_close_tag(html_input, i, &tag_name).unwrap_or(bytes.len());
                continue;
            }

            if let Some(ref skipping) = skip_tag {
                if is_close && &tag_name == skipping {
                    skip_tag = None;
                }
                continue;
            }

            if !is_close && tag_name == "head" && !self_closing_syntax {
                skip_tag = Some(tag_name);
                continue;
            }

            let target = if in_cell { &mut current_cell } else { &mut out };

            match (is_close, tag_name.as_str()) {
                (false, "h1") => target.push_str("\n# "),
                (false, "h2") => target.push_str("\n## "),
                (false, "h3") => target.push_str("\n### "),
                (false, "h4") => target.push_str("\n#### "),
                (false, "h5") => target.push_str("\n##### "),
                (false, "h6") => target.push_str("\n###### "),
                (true, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => target.push_str("\n\n"),
                (false, "ul") => list_stack.push((false, 0)),
                (false, "ol") => list_stack.push((true, 0)),
                (true, "ul" | "ol") => {
                    list_stack.pop();
                    target.push('\n');
                }
                (false, "li") => {
                    let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                    if let Some((true, count)) = list_stack.last_mut() {
                        *count += 1;
                        target.push_str(&format!("\n{indent}{count}. "));
                    } else {
                        target.push_str(&format!("\n{indent}- "));
                    }
                }
                (true, "li") => {}
                (false, "blockquote") => target.push_str("\n> "),
                (true, "blockquote") => target.push_str("\n\n"),
                (false, "strong" | "b") => target.push_str("**"),
                (true, "strong" | "b") => target.push_str("**"),
                (false, "em" | "i") => target.push('*'),
                (true, "em" | "i") => target.push('*'),
                (false, "pre") => {
                    in_pre = true;
                    target.push_str("\n```\n");
                }
                (true, "pre") => {
                    in_pre = false;
                    target.push_str("\n```\n\n");
                }
                (false, "code") if !in_pre => target.push('`'),
                (true, "code") if !in_pre => target.push('`'),
                (false, "a") => {
                    let href = extract_attr(inner, "href");
                    link_hrefs.push(href);
                    target.push('[');
                }
                (true, "a") => {
                    if let Some(Some(href)) = link_hrefs.pop() {
                        target.push_str(&format!("]({href})"));
                    } else {
                        target.push(']');
                    }
                }
                (false, "br") => target.push('\n'),
                (false, "hr") => target.push_str("\n---\n"),
                (true, "p" | "div") => target.push_str("\n\n"),
                (false, "table") => {
                    table_has_header_sep = false;
                    target.push('\n');
                }
                (true, "table") => target.push('\n'),
                (false, "tr") => {
                    in_table_row = true;
                    table_row_cells.clear();
                }
                (true, "tr") => {
                    if in_table_row && !table_row_cells.is_empty() {
                        out.push_str(&format!("| {} |\n", table_row_cells.join(" | ")));
                        if !table_has_header_sep {
                            let sep: Vec<&str> = table_row_cells.iter().map(|_| "---").collect();
                            out.push_str(&format!("| {} |\n", sep.join(" | ")));
                            table_has_header_sep = true;
                        }
                    }
                    in_table_row = false;
                    table_row_cells.clear();
                }
                (false, "td" | "th") => {
                    in_cell = true;
                    current_cell.clear();
                }
                (true, "td" | "th") => {
                    in_cell = false;
                    let cleaned = decode_html_entities(current_cell.trim())
                        .replace('|', "\\|")
                        .replace('\n', " ");
                    table_row_cells.push(cleaned);
                    current_cell.clear();
                }
                _ => {}
            }
            continue;
        }

        let ch = html_input[i..].chars().next().unwrap();
        if skip_tag.is_none() {
            let target = if in_cell { &mut current_cell } else { &mut out };
            if in_pre {
                target.push(ch);
            } else if ch == '\r' {
                // skip CR
            } else if ch == '\n' {
                if !target.ends_with('\n') && !target.ends_with(' ') {
                    target.push(' ');
                }
            } else {
                target.push(ch);
            }
        }
        i += ch.len_utf8();
    }

    let decoded = decode_html_entities(&out).replace('\u{00A0}', " ");
    let mut collapsed = String::new();
    let mut blank_run = 0usize;
    for raw_line in decoded.lines() {
        let mut line = raw_line.trim_end().to_string();
        // Convert Cocoa HTML Writer bullet paragraphs (`\t•\tText` or `• Text`) into Markdown bullets
        let trimmed_start = line.trim_start();
        if let Some(after_bullet) = trimmed_start.strip_prefix('•') {
            line = format!("- {}", after_bullet.trim_start());
        }
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !collapsed.is_empty() {
                collapsed.push('\n');
            }
        } else {
            blank_run = 0;
            collapsed.push_str(&line);
            collapsed.push('\n');
        }
    }
    let result = collapsed.trim().to_string();
    if result.is_empty() {
        String::new()
    } else {
        format!("{result}\n")
    }
}

pub fn convert_via_pandoc(
    input: &Path,
    from_fmt: &str,
    target_fmt: &str,
    output: &Path,
    staging_dir: &Path,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let Some(ref pandoc) = tools.pandoc else {
        return Err(EngineError::Unsupported(format!(
            "Pandoc is required to convert to/from {target_fmt}. Install via `brew install pandoc`."
        )));
    };

    let to = match target_fmt.to_ascii_uppercase().as_str() {
        "TXT" => "plain".to_string(),
        "MD" => "gfm".to_string(),
        "HTML" => "html5".to_string(),
        other => other.to_ascii_lowercase(),
    };

    let mut args = vec![
        "--sandbox".to_string(),
        "--standalone".to_string(),
        "--from".to_string(),
        from_fmt.to_string(),
        "--to".to_string(),
        to.clone(),
        "--output".to_string(),
        output.to_string_lossy().into_owned(),
        "--metadata".to_string(),
        "title=FLIP Document".to_string(),
    ];

    if to == "fb2" {
        let lua_filter = staging_dir.join("fb2-paragraphs.lua");
        let lua_src = "function Plain(b) return pandoc.Para(b.content) end\n\
            function CodeBlock(b)\n\
             local out = pandoc.List:new()\n\
             for line in (b.text .. '\\n'):gmatch('(.-)\\n') do\n\
              out:insert(pandoc.Para({pandoc.Str(line)}))\n\
             end\n\
             return out\n\
            end\n";
        fs::write(&lua_filter, lua_src)?;
        args.push("--lua-filter".to_string());
        args.push(lua_filter.to_string_lossy().into_owned());
    }
    if to == "html5" {
        args.push("--embed-resources".to_string());
    }
    args.push(input.to_string_lossy().into_owned());

    let (_out, err) = bridge::run_external_command(pandoc, &args, Some(staging_dir), 120, Some(cancel))?;
    let err_low = err.to_ascii_lowercase();
    if err_low.contains("could not fetch")
        || err_low.contains("could not convert image")
        || err_low.contains("could not find image")
    {
        return Err(EngineError::ConversionFailed(format!(
            "Document references remote/missing resources offline: {err}"
        )));
    }
    Ok(())
}

fn decode_text_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        if let Ok(s) = std::str::from_utf8(rest) {
            return s.to_string();
        }
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        if rest.len().is_multiple_of(2) {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        if rest.len().is_multiple_of(2) {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn read_html_file_safe(path: &Path, max_bytes: u64) -> Result<String, EngineError> {
    let meta = fs::metadata(path)?;
    if meta.len() > max_bytes {
        return Err(EngineError::ConversionFailed(format!(
            "HTML document exceeds {} MiB safety limit",
            max_bytes / (1024 * 1024)
        )));
    }
    let bytes = fs::read(path)?;
    Ok(decode_text_bytes(&bytes))
}

pub fn html_to_plain_text(html_input: &str) -> String {
    let mut out = String::with_capacity(html_input.len());
    let mut skip_tag: Option<String> = None;
    let mut in_pre = false;
    let mut list_stack: Vec<(bool, usize)> = Vec::new();
    let mut cell_index_in_row = 0usize;

    let bytes = html_input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if html_input[i..].starts_with("<!--") {
                if let Some(end_rel) = html_input[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                } else {
                    break;
                }
                continue;
            }
            if html_input[i..].starts_with("<![CDATA[") {
                let (cdata_end, next_i) = if let Some(end_rel) = html_input[i + 9..].find("]]>") {
                    (i + 9 + end_rel, i + 9 + end_rel + 3)
                } else {
                    (bytes.len(), bytes.len())
                };
                if skip_tag.is_none() {
                    out.push_str(&html_input[i + 9..cdata_end]);
                }
                i = next_i;
                continue;
            }
            if !matches!(
                bytes.get(i + 1),
                Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')
            ) {
                if skip_tag.is_none() {
                    out.push('<');
                }
                i += 1;
                continue;
            }

            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            while j < bytes.len() {
                let b = bytes[j];
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'>' {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                if skip_tag.is_none() {
                    out.push_str(&html_input[i..]);
                }
                break;
            }

            let raw_tag = html_input[i + 1..j].trim();
            i = j + 1;

            if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }

            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let self_closing_syntax = !is_close && has_xml_self_closing_slash(inner);
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if !is_close
                && matches!(tag_name.as_str(), "script" | "style" | "title")
                && !self_closing_syntax
            {
                i = find_raw_close_tag(html_input, i, &tag_name).unwrap_or(bytes.len());
                continue;
            }

            if let Some(ref skipping) = skip_tag {
                if is_close && &tag_name == skipping {
                    skip_tag = None;
                }
                continue;
            }

            if !is_close
                && matches!(
                    tag_name.as_str(),
                    "head" | "iframe" | "object" | "embed" | "noscript"
                )
                && !self_closing_syntax
            {
                skip_tag = Some(tag_name);
                continue;
            }

            match (is_close, tag_name.as_str()) {
                (false, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote") => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                (true, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "p" | "div" | "section" | "article") => {
                    out.push_str("\n\n");
                }
                (false, "ul") => list_stack.push((false, 0)),
                (false, "ol") => list_stack.push((true, 0)),
                (true, "ul" | "ol") => {
                    list_stack.pop();
                    out.push('\n');
                }
                (false, "li") => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                    if let Some((true, count)) = list_stack.last_mut() {
                        *count += 1;
                        out.push_str(&format!("{indent}{count}. "));
                    } else {
                        out.push_str(&format!("{indent}- "));
                    }
                }
                (true, "li") => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                (false, "pre") => {
                    in_pre = true;
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                (true, "pre") => {
                    in_pre = false;
                    out.push_str("\n\n");
                }
                (false, "br" | "hr") => out.push('\n'),
                (false, "tr") => {
                    cell_index_in_row = 0;
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                (true, "tr") => out.push('\n'),
                (false, "td" | "th") => {
                    if cell_index_in_row > 0 {
                        out.push('\t');
                    }
                    cell_index_in_row += 1;
                }
                _ => {}
            }
            continue;
        }

        let ch = html_input[i..].chars().next().unwrap();
        if skip_tag.is_none() {
            if in_pre {
                out.push(ch);
            } else if ch == '\r' {
                // skip CR
            } else if ch == '\n' {
                if !out.ends_with('\n') && !out.ends_with(' ') && !out.ends_with('\t') {
                    out.push(' ');
                }
            } else {
                out.push(ch);
            }
        }
        i += ch.len_utf8();
    }

    let decoded = decode_html_entities(&out).replace('\u{00A0}', " ");
    let mut collapsed = String::new();
    let mut blank_run = 0usize;
    for raw_line in decoded.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !collapsed.is_empty() {
                collapsed.push('\n');
            }
        } else {
            blank_run = 0;
            collapsed.push_str(line);
            collapsed.push('\n');
        }
    }
    let result = collapsed.trim().to_string();
    if result.is_empty() {
        String::new()
    } else {
        format!("{result}\n")
    }
}

fn find_tag_range_case_insensitive(input: &str, open_prefix: &str, close_tag: &str) -> Option<(usize, usize)> {
    let lower = input.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if lower[i..].starts_with("<!--") {
                if let Some(end_rel) = lower[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                    continue;
                }
                return None;
            }
            if open_prefix != "<script"
                && lower[i..].starts_with("<script")
                && matches!(bytes.get(i + 7), Some(b) if *b == b'>' || b.is_ascii_whitespace())
            {
                if let Some(gt_rel) = lower[i + 7..].find('>') {
                    let after_open = i + 7 + gt_rel + 1;
                    i = find_raw_close_tag(&lower, after_open, "script").unwrap_or(bytes.len());
                    continue;
                }
            }
            if open_prefix != "<style"
                && lower[i..].starts_with("<style")
                && matches!(bytes.get(i + 6), Some(b) if *b == b'>' || b.is_ascii_whitespace())
            {
                if let Some(gt_rel) = lower[i + 6..].find('>') {
                    let after_open = i + 6 + gt_rel + 1;
                    i = find_raw_close_tag(&lower, after_open, "style").unwrap_or(bytes.len());
                    continue;
                }
            }
            if lower[i..].starts_with(open_prefix) {
                let after = i + open_prefix.len();
                let next_b = bytes.get(after).copied();
                if matches!(next_b, Some(b) if b == b'>' || b.is_ascii_whitespace()) {
                    let mut j = after;
                    let mut quote: Option<u8> = None;
                    while j < bytes.len() {
                        let b = bytes[j];
                        if let Some(q) = quote {
                            if b == q {
                                quote = None;
                            }
                        } else if b == b'"' || b == b'\'' {
                            quote = Some(b);
                        } else if b == b'>' {
                            break;
                        }
                        j += 1;
                    }
                    if j >= bytes.len() {
                        return None;
                    }
                    let open_end = j + 1;
                    let close_rel = if close_tag == "</body>" {
                        lower[open_end..].rfind(close_tag)
                    } else {
                        lower[open_end..].find(close_tag)
                    };
                    if let Some(rel) = close_rel {
                        return Some((open_end, open_end + rel));
                    }
                    return Some((open_end, input.len()));
                }
            }
        }
        i += 1;
    }
    None
}

fn extract_html_title(html: &str) -> Option<String> {
    let (s, e) = find_tag_range_case_insensitive(html, "<title", "</title>")?;
    let raw = decode_html_entities(&html[s..e]);
    let trimmed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn extract_html_body_slice(html: &str) -> &str {
    if let Some((s, e)) = find_tag_range_case_insensitive(html, "<body", "</body>") {
        &html[s..e]
    } else {
        html
    }
}

#[derive(Debug, Clone)]
struct TocHeading {
    id: String,
    text: String,
}

fn is_safe_xhtml_url(raw: &str, allow_data_image: bool) -> bool {
    let compact: String = raw
        .chars()
        .filter(|c| !c.is_ascii_control() && !c.is_ascii_whitespace())
        .collect();
    if compact.is_empty() {
        return false;
    }
    let lower = compact.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("file:")
    {
        return false;
    }
    if lower.starts_with("data:") {
        return allow_data_image && lower.starts_with("data:image/");
    }
    true
}

fn is_valid_xml_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let valid_start = first.is_ascii_alphabetic() || first == '_' || first == ':';
    valid_start
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

fn allocate_unique_id(preferred: Option<&str>, used_ids: &mut HashSet<String>, seq: &mut usize) -> String {
    if let Some(raw) = preferred {
        let cleaned: String = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        if !cleaned.is_empty() {
            if used_ids.insert(cleaned.clone()) {
                return cleaned;
            }
            let mut suffix = 2usize;
            loop {
                let cand = format!("{cleaned}-{suffix}");
                if used_ids.insert(cand.clone()) {
                    return cand;
                }
                suffix += 1;
            }
        }
    }
    loop {
        *seq += 1;
        let cand = format!("sec-{seq}");
        if used_ids.insert(cand.clone()) {
            return cand;
        }
    }
}

fn normalize_html_to_xhtml5_body(html_input: &str) -> (String, Vec<TocHeading>) {
    let slice = extract_html_body_slice(html_input);
    let mut out = String::with_capacity(slice.len() + 64);
    let mut open_tags: Vec<String> = Vec::new();
    let mut skip_tag: Option<String> = None;
    let mut toc_headings: Vec<TocHeading> = Vec::new();
    let mut active_heading: Option<(String, String)> = None;
    let mut heading_seq = 0usize;
    let mut used_ids: HashSet<String> = HashSet::new();

    let bytes = slice.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if slice[i..].starts_with("<!--") {
                if let Some(end_rel) = slice[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                } else {
                    break;
                }
                continue;
            }
            if slice[i..].starts_with("<![CDATA[") {
                let (cdata_end, next_i) = if let Some(end_rel) = slice[i + 9..].find("]]>") {
                    (i + 9 + end_rel, i + 9 + end_rel + 3)
                } else {
                    (bytes.len(), bytes.len())
                };
                if skip_tag.is_none() {
                    let text_chunk = &slice[i + 9..cdata_end];
                    if let Some((_, ref mut htext)) = active_heading {
                        htext.push_str(text_chunk);
                    }
                    out.push_str(&xml_escape_text(text_chunk));
                }
                i = next_i;
                continue;
            }
            if !matches!(
                bytes.get(i + 1),
                Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')
            ) {
                if skip_tag.is_none() {
                    if let Some((_, ref mut htext)) = active_heading {
                        htext.push('<');
                    }
                    out.push_str("&lt;");
                }
                i += 1;
                continue;
            }

            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            while j < bytes.len() {
                let b = bytes[j];
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = Some(b);
                } else if b == b'>' {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                if skip_tag.is_none() {
                    let text = xml_escape_text(&decode_html_entities_inner(&slice[i..], true));
                    out.push_str(&text);
                }
                break;
            }

            let raw_tag = slice[i + 1..j].trim();
            i = j + 1;

            if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }

            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let self_closing_syntax = !is_close && has_xml_self_closing_slash(inner);
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if tag_name.is_empty() {
                continue;
            }

            if !is_close
                && matches!(tag_name.as_str(), "script" | "style" | "title")
                && !self_closing_syntax
            {
                i = find_raw_close_tag(slice, i, &tag_name).unwrap_or(bytes.len());
                continue;
            }

            if let Some(ref skipping) = skip_tag {
                if is_close && &tag_name == skipping {
                    skip_tag = None;
                }
                continue;
            }

            if !is_close
                && matches!(
                    tag_name.as_str(),
                    "head"
                        | "style"
                        | "script"
                        | "title"
                        | "iframe"
                        | "object"
                        | "embed"
                        | "noscript"
                        | "form"
                        | "base"
                        | "link"
                        | "meta"
                        | "svg"
                )
            {
                if !self_closing_syntax
                    && !matches!(tag_name.as_str(), "base" | "link" | "meta")
                {
                    skip_tag = Some(tag_name);
                }
                continue;
            }

            let allowed = matches!(
                tag_name.as_str(),
                "h1" | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "p"
                    | "div"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "aside"
                    | "main"
                    | "nav"
                    | "blockquote"
                    | "pre"
                    | "code"
                    | "ul"
                    | "ol"
                    | "li"
                    | "dl"
                    | "dt"
                    | "dd"
                    | "table"
                    | "thead"
                    | "tbody"
                    | "tfoot"
                    | "tr"
                    | "th"
                    | "td"
                    | "caption"
                    | "strong"
                    | "b"
                    | "em"
                    | "i"
                    | "u"
                    | "s"
                    | "del"
                    | "ins"
                    | "sub"
                    | "sup"
                    | "small"
                    | "mark"
                    | "span"
                    | "a"
                    | "br"
                    | "hr"
                    | "img"
                    | "input"
                    | "figure"
                    | "figcaption"
                    | "abbr"
                    | "cite"
                    | "q"
                    | "kbd"
                    | "samp"
                    | "var"
                    | "time"
            );
            if !allowed {
                continue;
            }

            let is_void = matches!(tag_name.as_str(), "br" | "hr" | "img" | "input");

            if is_close {
                if is_void {
                    continue;
                }
                if let Some(pos) = open_tags.iter().rposition(|t| t == &tag_name) {
                    while open_tags.len() > pos {
                        let popped = open_tags.pop().unwrap();
                        if matches!(popped.as_str(), "h1" | "h2" | "h3") {
                            if let Some((id, text)) = active_heading.take() {
                                let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                if !clean.is_empty() {
                                    toc_headings.push(TocHeading {
                                        id,
                                        text: clean,
                                    });
                                }
                            }
                        }
                        out.push_str(&format!("</{popped}>"));
                    }
                }
                continue;
            }

            // Auto-close unclosed <p> inside <p> or <li> directly inside <li>
            if tag_name == "p" && open_tags.last().is_some_and(|t| t == "p") {
                open_tags.pop();
                out.push_str("</p>");
            } else if tag_name == "li" && open_tags.last().is_some_and(|t| t == "li") {
                open_tags.pop();
                out.push_str("</li>");
            }

            let raw_attrs = parse_html_attrs(inner);
            let mut safe_attrs: Vec<(String, String)> = Vec::new();
            let mut seen_attr_names: HashSet<String> = HashSet::new();
            let mut element_id: Option<String> = None;

            for (k, v) in raw_attrs {
                if !is_valid_xml_attr_name(&k) || k.starts_with("on") || k == "style" || k == "srcset" {
                    continue;
                }
                if matches!(k.as_str(), "xmlns" | "xml:lang") || k.starts_with("xmlns:") {
                    continue;
                }
                let allowed_attr = matches!(
                    k.as_str(),
                    "id" | "class"
                        | "href"
                        | "src"
                        | "alt"
                        | "title"
                        | "colspan"
                        | "rowspan"
                        | "type"
                        | "checked"
                        | "disabled"
                        | "start"
                        | "lang"
                );
                if !allowed_attr || !seen_attr_names.insert(k.clone()) {
                    continue;
                }
                let decoded_val = decode_html_entities_inner(&v, true);
                if k == "href" && !is_safe_xhtml_url(&decoded_val, false) {
                    continue;
                }
                if k == "src" && !is_safe_xhtml_url(&decoded_val, true) {
                    continue;
                }
                if k == "id" {
                    if !decoded_val.trim().is_empty() {
                        let unique_id = allocate_unique_id(Some(&decoded_val), &mut used_ids, &mut heading_seq);
                        element_id = Some(unique_id.clone());
                        safe_attrs.push((k, unique_id));
                    }
                    continue;
                }
                let final_val = if matches!(k.as_str(), "disabled" | "checked") && decoded_val.is_empty() {
                    k.clone()
                } else {
                    decoded_val
                };
                safe_attrs.push((k, final_val));
            }

            if tag_name == "img" && !safe_attrs.iter().any(|(k, _)| k == "alt") {
                safe_attrs.push(("alt".to_string(), String::new()));
            }
            if tag_name == "img" && !safe_attrs.iter().any(|(k, _)| k == "src") {
                // Skip broken/blocked image tags that had disallowed file:// src
                continue;
            }

            if matches!(tag_name.as_str(), "h1" | "h2" | "h3") && !self_closing_syntax {
                let assigned_id = element_id.unwrap_or_else(|| {
                    let generated = allocate_unique_id(None, &mut used_ids, &mut heading_seq);
                    safe_attrs.push(("id".to_string(), generated.clone()));
                    generated
                });
                active_heading = Some((assigned_id, String::new()));
            }

            out.push('<');
            out.push_str(&tag_name);
            for (k, v) in &safe_attrs {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&xml_escape(v));
                out.push('"');
            }
            if is_void || self_closing_syntax {
                out.push_str("/>");
            } else {
                out.push('>');
                open_tags.push(tag_name);
            }
            continue;
        }

        let ch_start = i;
        while i < bytes.len() && bytes[i] != b'<' {
            i += 1;
        }
        if skip_tag.is_none() {
            let chunk = &slice[ch_start..i];
            let decoded = decode_html_entities_inner(chunk, true);
            if let Some((_, ref mut htext)) = active_heading {
                htext.push_str(&decoded);
            }
            out.push_str(&xml_escape_text(&decoded));
        }
    }

    while let Some(popped) = open_tags.pop() {
        if matches!(popped.as_str(), "h1" | "h2" | "h3") {
            if let Some((id, text)) = active_heading.take() {
                let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !clean.is_empty() {
                    toc_headings.push(TocHeading {
                        id,
                        text: clean,
                    });
                }
            }
        }
        out.push_str(&format!("</{popped}>"));
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        ("<p>&#160;</p>".to_string(), toc_headings)
    } else if !trimmed.starts_with('<') {
        (format!("<div>{trimmed}</div>"), toc_headings)
    } else {
        (trimmed.to_string(), toc_headings)
    }
}

pub fn write_epub_from_html(
    title: &str,
    html_input: &str,
    output: &Path,
) -> Result<(), EngineError> {
    let clean_title = {
        let candidate = title.trim();
        let extracted = extract_html_title(html_input);
        let chosen = if candidate.is_empty() || candidate == "FLIP Document" {
            extracted.as_deref().unwrap_or("FLIP Document")
        } else {
            candidate
        };
        let sanitized: String = chosen
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if sanitized.trim().is_empty() {
            "FLIP Document".to_string()
        } else {
            sanitized.trim().to_string()
        }
    };

    let (body_xhtml, toc_headings) = normalize_html_to_xhtml5_body(html_input);
    let book_uuid = Uuid::new_v4();
    let modified_utc = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let container_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\
  <rootfiles>\n\
    <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n\
  </rootfiles>\n\
</container>\n";

    let opf_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\">\n\
  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
    <dc:identifier id=\"bookid\">urn:uuid:{book_uuid}</dc:identifier>\n\
    <dc:title>{title_xml}</dc:title>\n\
    <dc:language>en</dc:language>\n\
    <meta property=\"dcterms:modified\">{modified_utc}</meta>\n\
  </metadata>\n\
  <manifest>\n\
    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n\
    <item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n\
    <item id=\"chapter1\" href=\"chapter.xhtml\" media-type=\"application/xhtml+xml\"/>\n\
  </manifest>\n\
  <spine>\n\
    <itemref idref=\"chapter1\"/>\n\
  </spine>\n\
</package>\n",
        title_xml = xml_escape(&clean_title),
    );

    let toc_items_xml = if toc_headings.is_empty() {
        format!(
            "        <li><a href=\"chapter.xhtml\">{}</a></li>\n",
            xml_escape(&clean_title)
        )
    } else {
        let mut items = String::new();
        for h in &toc_headings {
            items.push_str(&format!(
                "        <li><a href=\"chapter.xhtml#{}\">{}</a></li>\n",
                xml_escape(&h.id),
                xml_escape(&h.text)
            ));
        }
        items
    };

    let nav_xhtml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE html>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"en\" lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\"/>\n\
  <title>{title_xml}</title>\n\
  <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n\
</head>\n\
<body>\n\
  <nav epub:type=\"toc\" id=\"toc\">\n\
    <h1>Table of Contents</h1>\n\
    <ol>\n\
{toc_items_xml}\
    </ol>\n\
  </nav>\n\
</body>\n\
</html>\n",
        title_xml = xml_escape(&clean_title),
    );

    let style_css = "\
body {\n\
  font-family: -apple-system, Georgia, serif;\n\
  font-size: 1rem;\n\
  line-height: 1.6;\n\
  color: #111827;\n\
  margin: 1.2em auto;\n\
  max-width: 42em;\n\
  padding: 0 1em;\n\
}\n\
h1, h2, h3, h4, h5, h6 { font-family: -apple-system, Helvetica, Arial, sans-serif; line-height: 1.25; margin: 1.2em 0 0.5em; }\n\
p { margin: 0 0 0.8em; }\n\
pre, code { font-family: Menlo, monospace; font-size: 0.9em; background: #f3f4f6; }\n\
pre { padding: 0.75em; overflow-x: auto; border-radius: 4px; }\n\
blockquote { border-left: 3px solid #cbd5e1; margin: 0.8em 0; padding-left: 1em; color: #374151; }\n\
table { border-collapse: collapse; width: 100%; margin: 1em 0; }\n\
th, td { border: 1px solid #cbd5e1; padding: 0.35em 0.55em; text-align: left; }\n";

    let chapter_xhtml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE html>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"en\" lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\"/>\n\
  <title>{title_xml}</title>\n\
  <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n\
</head>\n\
<body>\n\
{body_xhtml}\n\
</body>\n\
</html>\n",
        title_xml = xml_escape(&clean_title),
    );

    let file = File::create(output)?;
    let mut zip = ZipWriter::new(file);

    // EPUB 3 OCF specification requires `mimetype` as entry 0, stored uncompressed without extra fields.
    let stored_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored_opts)
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to write EPUB mimetype: {e}")))?;
    zip.write_all(b"application/epub+zip")?;

    let deflate_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, content) in [
        ("META-INF/container.xml", container_xml.as_bytes()),
        ("OEBPS/content.opf", opf_xml.as_bytes()),
        ("OEBPS/nav.xhtml", nav_xhtml.as_bytes()),
        ("OEBPS/style.css", style_css.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter_xhtml.as_bytes()),
    ] {
        zip.start_file(name, deflate_opts)
            .map_err(|e| EngineError::ConversionFailed(format!("Failed to write EPUB entry `{name}`: {e}")))?;
        zip.write_all(content)?;
    }

    zip.finish()
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to finalize EPUB archive: {e}")))?;
    Ok(())
}

fn normalize_zip_path(raw: &str) -> String {
    let replaced = raw.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in replaced.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn percent_decode_href(raw: &str) -> String {
    let stripped = raw
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .trim();
    let bytes = stripped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve_zip_relative(base_dir: &str, href: &str) -> String {
    let decoded = percent_decode_href(href);
    if decoded.starts_with('/') || base_dir.is_empty() {
        normalize_zip_path(&decoded)
    } else {
        normalize_zip_path(&format!("{base_dir}/{decoded}"))
    }
}

fn xml_attr_str(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref() == name {
            return attr.unescape_value().ok().map(|s| s.into_owned());
        }
    }
    None
}

fn read_zip_entry_string(
    archive: &mut ZipArchive<File>,
    idx: usize,
    total_read: &mut u64,
    text_budget_remaining: &mut u64,
) -> Result<String, EngineError> {
    let mut entry = archive
        .by_index(idx)
        .map_err(|e| EngineError::ConversionFailed(format!("Corrupt EPUB entry: {e}")))?;
    let mut data = Vec::new();
    let mut buf = [0u8; 32768];
    loop {
        let n = entry.read(&mut buf)?;
        if n == 0 {
            break;
        }
        *total_read = total_read
            .checked_add(n as u64)
            .ok_or(EngineError::Policy(PolicyError::ContainerBombDetected))?;
        if *total_read > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(EngineError::Policy(PolicyError::ContainerBombDetected));
        }
        if (n as u64) > *text_budget_remaining {
            return Err(EngineError::ConversionFailed(
                "EPUB text content exceeds 32 MiB limit".to_string(),
            ));
        }
        *text_budget_remaining -= n as u64;
        data.extend_from_slice(&buf[..n]);
    }
    Ok(decode_text_bytes(&data))
}

#[derive(Debug, Clone)]
struct OpfManifestItem {
    path: String,
    media_type: String,
}

fn is_xhtml_item(item: &OpfManifestItem) -> bool {
    let mt = item.media_type.to_ascii_lowercase();
    let ext = item
        .path
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    mt.contains("xhtml")
        || mt.starts_with("text/html")
        || matches!(ext.as_str(), "xhtml" | "html" | "htm")
}

pub fn read_epub_to_html(source: &Path) -> Result<(String, String), EngineError> {
    let file = File::open(source)
        .map_err(|e| EngineError::ConversionFailed(format!("Failed to open EPUB: {e}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| EngineError::Policy(PolicyError::InvalidContainer(e.to_string())))?;

    if archive.len() > MAX_CONTAINER_ENTRIES {
        return Err(EngineError::Policy(PolicyError::ContainerTooManyEntries));
    }

    let mut exact_entries: BTreeMap<String, usize> = BTreeMap::new();
    let mut lower_entries: HashMap<String, usize> = HashMap::new();
    let mut declared_total: u64 = 0;

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| EngineError::Policy(PolicyError::InvalidContainer(e.to_string())))?;
        declared_total = declared_total
            .checked_add(entry.size())
            .ok_or(EngineError::Policy(PolicyError::ContainerBombDetected))?;
        if declared_total > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(EngineError::Policy(PolicyError::ContainerBombDetected));
        }
        if entry.is_dir() || entry.name().ends_with('/') {
            continue;
        }
        let norm = normalize_zip_path(entry.name());
        if !norm.is_empty() {
            lower_entries.entry(norm.to_ascii_lowercase()).or_insert(i);
            exact_entries.entry(norm).or_insert(i);
        }
    }

    let lookup_idx = |path: &str| -> Option<usize> {
        exact_entries
            .get(path)
            .copied()
            .or_else(|| lower_entries.get(&path.to_ascii_lowercase()).copied())
    };

    let mut total_read = 0u64;
    let mut text_budget = MAX_EPUB_TEXT_READ_BYTES;

    // 1. Locate rootfile .opf via META-INF/container.xml
    let mut opf_path: Option<String> = None;
    if let Some(container_idx) = lookup_idx("META-INF/container.xml") {
        let xml = read_zip_entry_string(&mut archive, container_idx, &mut total_read, &mut text_budget)?;
        let mut reader = Reader::from_str(&xml);
        let mut buf = Vec::new();
        let mut fallback_root: Option<String> = None;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    if e.local_name().as_ref() == b"rootfile" {
                        if let Some(fp) = xml_attr_str(e, b"full-path") {
                            let resolved = normalize_zip_path(&percent_decode_href(&fp));
                            let media_type = xml_attr_str(e, b"media-type").unwrap_or_default();
                            if media_type.eq_ignore_ascii_case("application/oebps-package+xml") {
                                opf_path = Some(resolved);
                                break;
                            } else if fallback_root.is_none() {
                                fallback_root = Some(resolved);
                            }
                        }
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        if opf_path.is_none() {
            opf_path = fallback_root;
        }
    }

    if opf_path.is_none() {
        opf_path = exact_entries
            .keys()
            .find(|k| k.to_ascii_lowercase().ends_with(".opf"))
            .cloned();
    }

    // 2. Parse .opf <metadata>, <manifest>, and <spine>
    let fallback_title = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("EPUB Document")
        .to_string();
    let mut book_title: Option<String> = None;
    let mut manifest_by_id: HashMap<String, OpfManifestItem> = HashMap::new();
    let mut manifest_xhtml_paths: Vec<String> = Vec::new();
    let mut nav_paths: HashSet<String> = HashSet::new();
    let mut spine_refs: Vec<(String, bool)> = Vec::new();

    if let Some(ref opf_p) = opf_path {
        if let Some(opf_idx) = lookup_idx(opf_p) {
            let opf_dir = opf_p.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let opf_xml =
                read_zip_entry_string(&mut archive, opf_idx, &mut total_read, &mut text_budget)?;
            let mut reader = Reader::from_str(&opf_xml);
            let mut buf = Vec::new();
            let mut in_metadata = false;
            let mut in_title = false;
            let mut title_acc = String::new();

            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) => {
                        let lname = e.local_name();
                        if lname.as_ref() == b"metadata" {
                            in_metadata = true;
                        } else if in_metadata && lname.as_ref() == b"title" {
                            in_title = true;
                            title_acc.clear();
                        } else if lname.as_ref() == b"item" {
                            if let (Some(id), Some(href)) =
                                (xml_attr_str(e, b"id"), xml_attr_str(e, b"href"))
                            {
                                let resolved = resolve_zip_relative(opf_dir, &href);
                                let media_type =
                                    xml_attr_str(e, b"media-type").unwrap_or_default();
                                let props = xml_attr_str(e, b"properties").unwrap_or_default();
                                let is_nav = props.split_whitespace().any(|p| p == "nav");
                                let item = OpfManifestItem {
                                    path: resolved.clone(),
                                    media_type,
                                };
                                if is_nav {
                                    nav_paths.insert(resolved.to_ascii_lowercase());
                                } else if is_xhtml_item(&item) {
                                    manifest_xhtml_paths.push(resolved);
                                }
                                manifest_by_id.insert(id, item);
                            }
                        } else if lname.as_ref() == b"itemref" {
                            if let Some(idref) = xml_attr_str(e, b"idref") {
                                let linear = xml_attr_str(e, b"linear")
                                    .map(|v| !v.eq_ignore_ascii_case("no"))
                                    .unwrap_or(true);
                                spine_refs.push((idref, linear));
                            }
                        }
                    }
                    Ok(Event::Empty(ref e)) => {
                        let lname = e.local_name();
                        if lname.as_ref() == b"item" {
                            if let (Some(id), Some(href)) =
                                (xml_attr_str(e, b"id"), xml_attr_str(e, b"href"))
                            {
                                let resolved = resolve_zip_relative(opf_dir, &href);
                                let media_type =
                                    xml_attr_str(e, b"media-type").unwrap_or_default();
                                let props = xml_attr_str(e, b"properties").unwrap_or_default();
                                let is_nav = props.split_whitespace().any(|p| p == "nav");
                                let item = OpfManifestItem {
                                    path: resolved.clone(),
                                    media_type,
                                };
                                if is_nav {
                                    nav_paths.insert(resolved.to_ascii_lowercase());
                                } else if is_xhtml_item(&item) {
                                    manifest_xhtml_paths.push(resolved);
                                }
                                manifest_by_id.insert(id, item);
                            }
                        } else if lname.as_ref() == b"itemref" {
                            if let Some(idref) = xml_attr_str(e, b"idref") {
                                let linear = xml_attr_str(e, b"linear")
                                    .map(|v| !v.eq_ignore_ascii_case("no"))
                                    .unwrap_or(true);
                                spine_refs.push((idref, linear));
                            }
                        }
                    }
                    Ok(Event::Text(ref t)) => {
                        if in_title {
                            match t.unescape() {
                                Ok(decoded) => title_acc.push_str(&decoded),
                                Err(_) => title_acc.push_str(&decode_html_entities(&String::from_utf8_lossy(t.as_ref()))),
                            }
                        }
                    }
                    Ok(Event::CData(ref t)) => {
                        if in_title {
                            title_acc.push_str(&String::from_utf8_lossy(t.as_ref()));
                        }
                    }
                    Ok(Event::End(ref e)) => {
                        let lname = e.local_name();
                        if lname.as_ref() == b"metadata" {
                            in_metadata = false;
                        } else if lname.as_ref() == b"title" && in_title {
                            in_title = false;
                            let cleaned = title_acc.trim();
                            if !cleaned.is_empty() && book_title.is_none() {
                                book_title = Some(cleaned.to_string());
                            }
                        }
                    }
                    Ok(Event::Eof) | Err(_) => break,
                    _ => {}
                }
                buf.clear();
            }
        }
    }

    // 3. Determine reading order: <spine> -> manifest -> lexical XHTML fallback if spine incomplete
    let mut linear_chapters: Vec<String> = Vec::new();
    let mut nonlinear_chapters: Vec<String> = Vec::new();
    let mut seen_lower: HashSet<String> = HashSet::new();
    let mut spine_incomplete = spine_refs.is_empty();

    for (idref, is_linear) in &spine_refs {
        if let Some(item) = manifest_by_id.get(idref) {
            if lookup_idx(&item.path).is_some() && is_xhtml_item(item) {
                let low = item.path.to_ascii_lowercase();
                if seen_lower.insert(low) {
                    if *is_linear {
                        linear_chapters.push(item.path.clone());
                    } else {
                        nonlinear_chapters.push(item.path.clone());
                    }
                }
            } else {
                spine_incomplete = true;
            }
        } else {
            spine_incomplete = true;
        }
    }

    let mut chapter_paths = if !linear_chapters.is_empty() {
        linear_chapters
    } else {
        nonlinear_chapters
    };

    if chapter_paths.is_empty() || spine_incomplete {
        for p in manifest_xhtml_paths {
            if lookup_idx(&p).is_some() && seen_lower.insert(p.to_ascii_lowercase()) {
                chapter_paths.push(p);
            }
        }
        // Lexical XHTML fallback across all non-META-INF, non-nav content XHTML files
        for p in exact_entries.keys() {
            let low = p.to_ascii_lowercase();
            if low.starts_with("meta-inf/") {
                continue;
            }
            if !matches!(
                low.rsplit('.').next(),
                Some("xhtml" | "html" | "htm")
            ) {
                continue;
            }
            if nav_paths.contains(&low) {
                continue;
            }
            if seen_lower.insert(low) {
                chapter_paths.push(p.clone());
            }
        }
        if chapter_paths.is_empty() {
            for p in exact_entries.keys() {
                let low = p.to_ascii_lowercase();
                if !low.starts_with("meta-inf/")
                    && matches!(low.rsplit('.').next(), Some("xhtml" | "html" | "htm"))
                    && seen_lower.insert(low)
                {
                    chapter_paths.push(p.clone());
                }
            }
        }
    }

    if chapter_paths.is_empty() {
        return Err(EngineError::ConversionFailed(
            "EPUB contains no readable XHTML chapters".to_string(),
        ));
    }

    // 4. Extract chapter XHTML body contents cleanly
    let mut chapters_html = String::new();
    for ch_path in &chapter_paths {
        let Some(idx) = lookup_idx(ch_path) else {
            continue;
        };
        let raw_xhtml =
            read_zip_entry_string(&mut archive, idx, &mut total_read, &mut text_budget)?;
        let (clean_body, _) = normalize_html_to_xhtml5_body(&raw_xhtml);
        chapters_html.push_str("<section class=\"chapter\">\n");
        chapters_html.push_str(&clean_body);
        chapters_html.push_str("\n</section>\n");
    }

    if chapters_html.trim().is_empty() {
        return Err(EngineError::ConversionFailed(
            "EPUB chapters contained no extractable content".to_string(),
        ));
    }

    let final_title = book_title.unwrap_or(fallback_title);
    let combined = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:11.5pt;line-height:1.5;color:#111;}}h1,h2,h3{{margin:14pt 0 6pt 0;}}pre,code{{font-family:Menlo,monospace;font-size:10pt;background:#f4f4f6;}}table{{border-collapse:collapse;width:100%;}}th,td{{border:1px solid #ccc;padding:4pt 6pt;}}section.chapter{{margin-bottom:16pt;}}</style>\
        </head><body>\n{chapters_html}</body></html>",
        html_escape(&final_title)
    );
    Ok((final_title, combined))
}

pub fn convert(
    source: &Path,
    format: &str,
    staging_dir: &Path,
    prefs: &AppPreferences,
    tools: &DetectedTools,
    cancel: &Arc<AtomicBool>,
    unit_progress: &dyn Fn(u8),
) -> Result<Vec<PathBuf>, EngineError> {
    let ext = ext_lower(source);
    let fmt = format.trim().to_ascii_uppercase();
    let out_ext = format_extension(&fmt);
    let output = staging_dir.join(format!("output.{out_ext}"));
    let source_title = source
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Document");

    unit_progress(25);

    // 1) Apple iWork (.pages, .numbers, .key)
    if matches!(ext.as_str(), "pages" | "numbers" | "key") {
        bridge::convert_doc(source, &fmt, &output, &prefs.pdf_page_size, Some(cancel))?;
        return Ok(vec![output]);
    }

    // 2) Render document to images via intermediate PDF
    if matches!(fmt.as_str(), "PNG" | "JPG" | "WEBP" | "HEIC") {
        let inter_pdf = staging_dir.join("intermediate.pdf");
        let pdf_outputs = convert(
            source,
            "PDF",
            staging_dir,
            prefs,
            tools,
            cancel,
            &|_| {},
        )?;
        let pdf_path = pdf_outputs.into_iter().next().unwrap_or(inter_pdf);
        return pdf::convert(&pdf_path, &fmt, staging_dir, prefs, tools, cancel, unit_progress);
    }

    // 3) Markdown source: render via pulldown-cmark -> styled HTML first
    let prepared_input: PathBuf;
    let prepared_ext: String;
    if matches!(ext.as_str(), "md" | "markdown") {
        let meta = fs::metadata(source)?;
        if meta.len() > MAX_TEXT_READ_BYTES {
            return Err(EngineError::ConversionFailed(
                "Markdown file exceeds 2 MiB text limit".to_string(),
            ));
        }
        let md_text = fs::read_to_string(source)?;
        let html_str = markdown_to_html(source_title, &md_text);
        let html_path = staging_dir.join("markdown.html");
        fs::write(&html_path, &html_str)?;
        if fmt == "HTML" {
            return Ok(vec![html_path]);
        }
        if fmt == "EPUB" {
            write_epub_from_html(source_title, &html_str, &output)?;
            return Ok(vec![output]);
        }
        prepared_input = html_path;
        prepared_ext = "html".to_string();
    } else {
        prepared_input = source.to_path_buf();
        prepared_ext = ext.clone();
    }

    // 4) Native pure-Rust EPUB 3 reader (EPUB -> TXT/MD/HTML/RTF/DOCX/ODT/PDF, or FB2/RST via Pandoc)
    if prepared_ext == "epub" {
        let (_book_title, combined_html) = read_epub_to_html(&prepared_input)?;
        if fmt == "HTML" {
            fs::write(&output, combined_html)?;
            return Ok(vec![output]);
        }
        if fmt == "MD" {
            let md = html_to_markdown(&combined_html);
            fs::write(&output, md)?;
            return Ok(vec![output]);
        }
        if fmt == "TXT" {
            let txt = html_to_plain_text(&combined_html);
            fs::write(&output, txt)?;
            return Ok(vec![output]);
        }
        let html_mid = staging_dir.join("epub-extracted.html");
        fs::write(&html_mid, combined_html)?;
        if matches!(fmt.as_str(), "FB2" | "RST") {
            convert_via_pandoc(&html_mid, "html", &fmt, &output, staging_dir, tools, cancel)?;
            return Ok(vec![output]);
        }
        bridge::convert_doc(&html_mid, &fmt, &output, &prefs.pdf_page_size, Some(cancel))?;
        return Ok(vec![output]);
    }

    // 5) Native pure-Rust EPUB 3 writer (TXT, HTML, RTF, DOC, DOCX, ODT, FB2/RST -> EPUB)
    if fmt == "EPUB" {
        let html_for_epub = if prepared_ext == "txt" {
            let meta = fs::metadata(&prepared_input)?;
            if meta.len() > MAX_TEXT_READ_BYTES {
                return Err(EngineError::ConversionFailed(
                    "Text file exceeds 2 MiB limit".to_string(),
                ));
            }
            let txt = fs::read_to_string(&prepared_input)?;
            plain_text_to_html(source_title, &txt)
        } else if matches!(prepared_ext.as_str(), "html" | "htm") {
            read_html_file_safe(&prepared_input, MAX_HTML_READ_BYTES)?
        } else if matches!(prepared_ext.as_str(), "fb2" | "rst") {
            let html_mid = staging_dir.join("pandoc-to-epub.html");
            convert_via_pandoc(
                &prepared_input,
                &prepared_ext,
                "HTML",
                &html_mid,
                staging_dir,
                tools,
                cancel,
            )?;
            read_html_file_safe(&html_mid, MAX_HTML_READ_BYTES)?
        } else {
            let html_mid = staging_dir.join("doc-to-epub.html");
            bridge::convert_doc(
                &prepared_input,
                "HTML",
                &html_mid,
                &prefs.pdf_page_size,
                Some(cancel),
            )?;
            read_html_file_safe(&html_mid, MAX_HTML_READ_BYTES)?
        };
        write_epub_from_html(source_title, &html_for_epub, &output)?;
        return Ok(vec![output]);
    }

    // 6) Pandoc-only formats (FB2, RST) either as source or target
    if matches!(prepared_ext.as_str(), "fb2" | "rst") || matches!(fmt.as_str(), "FB2" | "RST") {
        let from_pandoc = match prepared_ext.as_str() {
            "md" | "markdown" => "markdown",
            "htm" | "html" => "html",
            "txt" => "plain",
            other => other,
        };
        if matches!(fmt.as_str(), "FB2" | "RST") && prepared_ext == "txt" {
            let txt_raw = fs::read_to_string(&prepared_input)?;
            let html_path = staging_dir.join("txt-wrap.html");
            fs::write(&html_path, plain_text_to_html("FLIP Document", &txt_raw))?;
            convert_via_pandoc(&html_path, "html", &fmt, &output, staging_dir, tools, cancel)?;
            return Ok(vec![output]);
        }
        if matches!(prepared_ext.as_str(), "fb2" | "rst") && fmt == "PDF" {
            let html_mid = staging_dir.join("pandoc-mid.html");
            convert_via_pandoc(&prepared_input, from_pandoc, "HTML", &html_mid, staging_dir, tools, cancel)?;
            bridge::convert_doc(&html_mid, "PDF", &output, &prefs.pdf_page_size, Some(cancel))?;
            return Ok(vec![output]);
        }
        convert_via_pandoc(&prepared_input, from_pandoc, &fmt, &output, staging_dir, tools, cancel)?;
        return Ok(vec![output]);
    }

    // 7) Target is MD
    if fmt == "MD" {
        if tools.pandoc.is_some() && matches!(prepared_ext.as_str(), "html" | "htm" | "docx" | "odt" | "rtf") {
            let from_pd = if prepared_ext == "htm" { "html" } else { prepared_ext.as_str() };
            if convert_via_pandoc(&prepared_input, from_pd, "MD", &output, staging_dir, tools, cancel).is_ok() {
                return Ok(vec![output]);
            }
        }
        if prepared_ext == "txt" {
            let raw = fs::read_to_string(&prepared_input)?;
            fs::write(&output, raw)?;
            return Ok(vec![output]);
        }
        let html_path = if matches!(prepared_ext.as_str(), "html" | "htm") {
            prepared_input.clone()
        } else {
            let tmp_html = staging_dir.join("to-md.html");
            bridge::convert_doc(&prepared_input, "HTML", &tmp_html, &prefs.pdf_page_size, Some(cancel))?;
            tmp_html
        };
        let html_raw = fs::read_to_string(&html_path)?;
        let md = html_to_markdown(&html_raw);
        fs::write(&output, md)?;
        return Ok(vec![output]);
    }

    // 8) Native AppKit / NSAttributedString / CoreText bridge (TXT, HTML, RTF, DOC, DOCX, ODT -> PDF/DOCX/RTF/HTML/TXT/ODT)
    if prepared_ext == "txt" {
        let meta = fs::metadata(&prepared_input)?;
        if meta.len() > MAX_TEXT_READ_BYTES {
            return Err(EngineError::ConversionFailed(
                "Text file exceeds 2 MiB limit".to_string(),
            ));
        }
        if fmt == "HTML" {
            let txt = fs::read_to_string(&prepared_input)?;
            let title = source
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Document");
            fs::write(&output, plain_text_to_html(title, &txt))?;
            return Ok(vec![output]);
        }
    }

    bridge::convert_doc(
        &prepared_input,
        &fmt,
        &output,
        &prefs.pdf_page_size,
        Some(cancel),
    )?;
    Ok(vec![output])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_markdown_handles_attributes_cocoa_bullets_links_and_header() {
        let html = r#"<!DOCTYPE html>
<html>
<head><title>Hidden Head</title><style>.p1{color:red}</style></head>
<body>
<header><h1 style="font-size:24px">Page Header</h1></header>
<p class="p1">Intro with <a href="https://example.com">Link &amp; More</a> and <strong class="b">bold</strong>.</p>
<ul class="list"><li class="li1">Alpha</li><li class="li1">Beta</li></ul>
<p class="p2"><span class="Apple-tab-span">	</span>•<span class="Apple-tab-span">	</span>Cocoa Bullet</p>
<table><tr><th>Name</th><th>City</th></tr><tr><td>Ada</td><td>Milano</td></tr></table>
</body>
</html>"#;
        let md = html_to_markdown(html);
        assert!(!md.contains("Hidden Head"));
        assert!(!md.contains("color:red"));
        assert!(md.contains("# Page Header"), "missing heading in:\n{md}");
        assert!(md.contains("[Link & More](https://example.com)"), "missing link in:\n{md}");
        assert!(md.contains("**bold**"), "missing bold in:\n{md}");
        assert!(md.contains("- Alpha") && md.contains("- Beta"), "missing ul bullets in:\n{md}");
        assert!(md.contains("- Cocoa Bullet"), "missing cocoa bullet in:\n{md}");
        assert!(md.contains("| Name | City |") && md.contains("| Ada | Milano |"), "missing table in:\n{md}");
    }

    fn assert_valid_xml(raw: &str, label: &str) {
        let mut reader = Reader::from_str(raw);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("{label} is not valid XML: {e}\n{raw}"),
            }
            buf.clear();
        }
    }

    #[test]
    fn epub3_writer_and_reader_produce_valid_container_and_wellformed_xhtml5() {
        let dir = tempfile::tempdir().unwrap();
        let epub_path = dir.path().join("book.epub");
        let dirty_html = r#"<!DOCTYPE html>
<html>
<head><title>Caffè Guide</title><style>body{color:red}</style><script>evil()</script></head>
<body>
<h1>Chapter One: Caffè &amp; Espresso</h1>
<p>First line<br>Second&nbsp;line with &mdash; dash and &euro;10.</p>
<hr>
<p>Check <a href="file:///etc/passwd">blocked</a> and <a href="https://example.com">safe</a>.</p>
<ul><li>Item A<li>Item B</ul>
</body>
</html>"#;

        write_epub_from_html("Caffè Guide", dirty_html, &epub_path).unwrap();

        let file = File::open(&epub_path).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();

        {
            let mut mimetype_entry = archive.by_index(0).unwrap();
            assert_eq!(mimetype_entry.name(), "mimetype");
            assert_eq!(mimetype_entry.compression(), CompressionMethod::Stored);
            let mut m = String::new();
            mimetype_entry.read_to_string(&mut m).unwrap();
            assert_eq!(m, "application/epub+zip");
        }

        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().to_string());
        }
        assert!(names.contains(&"META-INF/container.xml".to_string()));
        assert!(names.contains(&"OEBPS/content.opf".to_string()));
        assert!(names.contains(&"OEBPS/nav.xhtml".to_string()));
        assert!(names.contains(&"OEBPS/style.css".to_string()));
        assert!(names.contains(&"OEBPS/chapter.xhtml".to_string()));

        for xml_name in [
            "META-INF/container.xml",
            "OEBPS/content.opf",
            "OEBPS/nav.xhtml",
            "OEBPS/chapter.xhtml",
        ] {
            let mut s = String::new();
            archive.by_name(xml_name).unwrap().read_to_string(&mut s).unwrap();
            assert_valid_xml(&s, xml_name);
        }

        let mut opf = String::new();
        archive.by_name("OEBPS/content.opf").unwrap().read_to_string(&mut opf).unwrap();
        assert!(opf.contains("dc:identifier"));
        assert!(opf.contains("dc:title"));
        assert!(opf.contains("dc:language"));
        assert!(opf.contains("dcterms:modified"));
        assert!(opf.contains("<manifest>"));
        assert!(opf.contains("<spine>"));

        let mut nav = String::new();
        archive.by_name("OEBPS/nav.xhtml").unwrap().read_to_string(&mut nav).unwrap();
        assert!(nav.contains("epub:type=\"toc\""));
        assert!(nav.contains("Chapter One: Caffè"));

        let mut chap = String::new();
        archive.by_name("OEBPS/chapter.xhtml").unwrap().read_to_string(&mut chap).unwrap();
        assert!(!chap.contains("evil()"));
        assert!(!chap.contains("file:///etc/passwd"));
        assert!(chap.contains("https://example.com"));

        let (title, combined_html) = read_epub_to_html(&epub_path).unwrap();
        assert_eq!(title, "Caffè Guide");
        let txt = html_to_plain_text(&combined_html);
        assert!(txt.contains("Chapter One: Caffè & Espresso"));
        assert!(txt.contains("Second line with — dash and €10."));
        assert!(txt.contains("- Item A"));
        assert!(txt.contains("- Item B"));

        let md = html_to_markdown(&combined_html);
        assert!(md.contains("# Chapter One: Caffè & Espresso"));
        assert!(md.contains("[safe](https://example.com)"));
    }

    #[test]
    fn epub_reader_respects_spine_order_and_lexical_fallback_when_spine_incomplete() {
        let dir = tempfile::tempdir().unwrap();

        // 1) Custom spine order (chapter 2 before chapter 1)
        let ordered_epub = dir.path().join("spine_order.epub");
        {
            let mut z = ZipWriter::new(File::create(&ordered_epub).unwrap());
            z.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            let def = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            z.start_file("META-INF/container.xml", def).unwrap();
            z.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/pkg.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            z.start_file("OPS/pkg.opf", def).unwrap();
            z.write_all(br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Ordered</dc:title></metadata><manifest><item id="c1" href="Text/01.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="Text/02.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="c2"/><itemref idref="c1"/></spine></package>"#).unwrap();
            z.start_file("OPS/nav.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>NAV_SHOULD_BE_SKIPPED</p></body></html>"#).unwrap();
            z.start_file("OPS/Text/01.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>ALPHA_SECOND</p></body></html>"#).unwrap();
            z.start_file("OPS/Text/02.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>BETA_FIRST</p></body></html>"#).unwrap();
            z.finish().unwrap();
        }
        let (title, html_out) = read_epub_to_html(&ordered_epub).unwrap();
        assert_eq!(title, "Ordered");
        let plain = html_to_plain_text(&html_out);
        let pos_first = plain.find("BETA_FIRST").unwrap();
        let pos_second = plain.find("ALPHA_SECOND").unwrap();
        assert!(pos_first < pos_second, "spine order not respected:\n{plain}");
        assert!(!plain.contains("NAV_SHOULD_BE_SKIPPED"));

        // 2) Incomplete spine -> lexical XHTML fallback picks up unreferenced chapters cleanly
        let broken_spine = dir.path().join("broken_spine.epub");
        {
            let mut z = ZipWriter::new(File::create(&broken_spine).unwrap());
            z.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            let def = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            z.start_file("META-INF/container.xml", def).unwrap();
            z.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
            z.start_file("OEBPS/content.opf", def).unwrap();
            z.write_all(br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Fallback</dc:title></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="missing_id"/></spine></package>"#).unwrap();
            z.start_file("OEBPS/nav.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>NAV_IGNORED</p></body></html>"#).unwrap();
            z.start_file("OEBPS/ch02.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>CHAPTER_TWO</p></body></html>"#).unwrap();
            z.start_file("OEBPS/ch01.xhtml", def).unwrap();
            z.write_all(br#"<html><body><p>CHAPTER_ONE</p></body></html>"#).unwrap();
            z.finish().unwrap();
        }
        let (_, fb_html) = read_epub_to_html(&broken_spine).unwrap();
        let fb_plain = html_to_plain_text(&fb_html);
        assert!(fb_plain.find("CHAPTER_ONE").unwrap() < fb_plain.find("CHAPTER_TWO").unwrap());
        assert!(!fb_plain.contains("NAV_IGNORED"));
    }

    #[test]
    fn html_comment_apostrophes_scripts_cdata_and_duplicate_heading_ids_handled_safely() {
        let dir = tempfile::tempdir().unwrap();
        let epub_path = dir.path().join("edge_html.epub");
        let nasty_html = r#"<!DOCTYPE html>
<html>
<head>
  <!-- <title>Fake Comment Title</title> -->
  <title>Real &amp; Clean Title</title>
  <script>var fake = "<body>"; // don't break here</script>
</head>
<body>
  <!-- Don't break on apostrophe in comment -->
  <h1 id="sec-2">First Heading</h1>
  <h2>Second Heading</h2>
  <script>
    if (a < b) {
      // don't break on '<' + apostrophe!
    }
  </script>
  <script src=https://example.com/app/>EVIL_LEAK()</script>
  <style>/* it's a < style */ p { color: red; }</style>
  <p>AFTER_COMMENT_AND_SCRIPT: 1 < 2 and <![CDATA[a < b & c]]></p>
  <p><a href="java&#9;script:alert(1)">blocked_js</a> and <a href="https://example.org">ok</a></p>
</body>
</html>"#;

        write_epub_from_html("FLIP Document", nasty_html, &epub_path).unwrap();

        let file = File::open(&epub_path).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();

        let mut chap = String::new();
        archive.by_name("OEBPS/chapter.xhtml").unwrap().read_to_string(&mut chap).unwrap();
        assert_valid_xml(&chap, "OEBPS/chapter.xhtml");

        let mut nav = String::new();
        archive.by_name("OEBPS/nav.xhtml").unwrap().read_to_string(&mut nav).unwrap();
        assert_valid_xml(&nav, "OEBPS/nav.xhtml");

        assert!(!chap.contains("EVIL_LEAK"));
        assert!(!chap.contains("Don't break on apostrophe"));
        assert!(!chap.contains("alert(1)"));
        assert!(chap.contains("AFTER_COMMENT_AND_SCRIPT"));
        assert!(chap.contains("1 &lt; 2"));
        assert!(chap.contains("a &lt; b &amp; c"));
        assert!(chap.contains("https://example.org"));

        // Heading IDs must never collide even when input already claimed id="sec-2"
        assert_eq!(chap.matches("id=\"sec-2\"").count(), 1, "duplicate sec-2 in:\n{chap}");
        assert!(chap.contains("id=\"sec-1\""), "expected sec-1 for Second Heading:\n{chap}");
        assert!(nav.contains("chapter.xhtml#sec-2") && nav.contains("chapter.xhtml#sec-1"));

        let (title, combined) = read_epub_to_html(&epub_path).unwrap();
        assert_eq!(title, "Real & Clean Title");

        let plain = html_to_plain_text(&combined);
        assert!(plain.contains("AFTER_COMMENT_AND_SCRIPT: 1 < 2 and a < b & c"), "plain={plain}");
        assert!(!plain.contains("EVIL_LEAK"));

        let md = html_to_markdown(&combined);
        assert!(md.contains("AFTER_COMMENT_AND_SCRIPT: 1 < 2 and a < b & c"), "md={md}");
        assert!(!md.contains("EVIL_LEAK"));
    }
}
