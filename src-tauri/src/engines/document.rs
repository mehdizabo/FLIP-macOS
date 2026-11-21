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

pub fn html_to_markdown_with_fallback(html_input: &str, fallback_title: &str) -> String {
    let md = html_to_markdown(html_input);
    if !md.trim().is_empty() {
        return md;
    }
    if html_input.trim().is_empty() {
        return String::new();
    }
    let heading = resolve_html_fallback_heading(html_input, fallback_title);
    let fence = if html_input.contains("```") { "~~~~" } else { "```" };
    let body = html_input.trim();
    if let Some(title) = heading {
        format!("# {title}\n\n{fence}html\n{body}\n{fence}\n")
    } else {
        format!("{fence}html\n{body}\n{fence}\n")
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

    let (_out, err) = bridge::run_external_command(pandoc, &args, Some(staging_dir), 120, Some(cancel))
        .map_err(|e| match e {
            bridge::BridgeError::Cancelled => EngineError::Cancelled,
            other => EngineError::Bridge(other),
        })?;
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

pub fn html_to_plain_text_with_fallback(html_input: &str, fallback_title: &str) -> String {
    let txt = html_to_plain_text(html_input);
    if !txt.trim().is_empty() {
        return txt;
    }
    if html_input.trim().is_empty() {
        return String::new();
    }
    let heading = resolve_html_fallback_heading(html_input, fallback_title);
    let body = html_input.trim();
    if let Some(title) = heading {
        format!("{title}\n\n{body}\n")
    } else {
        format!("{body}\n")
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
        let ch = lower[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
    None
}

pub fn extract_html_title(html_input: &str) -> Option<String> {
    let lower = html_input.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if lower[i..].starts_with("<!--") {
                if let Some(end_rel) = lower[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                    continue;
                }
                return None;
            }
            if lower[i..].starts_with("<script")
                && matches!(bytes.get(i + 7), Some(b) if *b == b'>' || b.is_ascii_whitespace())
            {
                if let Some(gt_rel) = lower[i + 7..].find('>') {
                    let after_open = i + 7 + gt_rel + 1;
                    i = find_raw_close_tag(&lower, after_open, "script").unwrap_or(bytes.len());
                    continue;
                }
            }
            if lower[i..].starts_with("<style")
                && matches!(bytes.get(i + 6), Some(b) if *b == b'>' || b.is_ascii_whitespace())
            {
                if let Some(gt_rel) = lower[i + 6..].find('>') {
                    let after_open = i + 6 + gt_rel + 1;
                    i = find_raw_close_tag(&lower, after_open, "style").unwrap_or(bytes.len());
                    continue;
                }
            }
            if lower[i..].starts_with("<title")
                && matches!(bytes.get(i + 6), Some(b) if *b == b'>' || b.is_ascii_whitespace())
            {
                let mut j = i + 6;
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
                if has_xml_self_closing_slash(&html_input[i + 1..j]) {
                    i = open_end;
                    continue;
                }
                let mut close_search = open_end;
                let mut close_start: Option<usize> = None;
                while let Some(rel) = lower[close_search..].find("</title") {
                    let candidate = close_search + rel;
                    let after = candidate + "</title".len();
                    let next_b = bytes.get(after).copied();
                    if matches!(next_b, Some(b) if b == b'>' || b.is_ascii_whitespace()) {
                        if lower[after..].contains('>') {
                            close_start = Some(candidate);
                        }
                        break;
                    }
                    close_search = after;
                }
                let close_idx = close_start?;
                let raw_inner = &html_input[open_end..close_idx];
                let mut inner_text = String::with_capacity(raw_inner.len());
                let mut rest = raw_inner;
                while let Some(lt) = rest.find('<') {
                    inner_text.push_str(&rest[..lt]);
                    if let Some(gt) = rest[lt + 1..].find('>') {
                        rest = &rest[lt + 1 + gt + 1..];
                    } else {
                        rest = "";
                        break;
                    }
                }
                inner_text.push_str(rest);
                let decoded = decode_html_entities(&inner_text);
                let cleaned = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
                return if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                };
            }
        }
        let ch = lower[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
    None
}

fn resolve_html_fallback_heading(html_input: &str, fallback_title: &str) -> Option<String> {
    extract_html_title(html_input).or_else(|| {
        let t = fallback_title.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
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

// =========================================================================
// Native Pure-Rust FictionBook 2.0 (FB2) Reader & Writer
// =========================================================================

fn read_text_bounded_with_utf16(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<String, EngineError> {
    let meta = fs::metadata(path)?;
    if meta.len() > max_bytes {
        return Err(EngineError::ConversionFailed(format!(
            "{label} file exceeds size limit ({} bytes > {max_bytes} bytes)",
            meta.len()
        )));
    }
    let bytes = fs::read(path)?;
    Ok(decode_text_bytes(&bytes))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fb2OpenTag {
    Section,
    Title(u8),
    TitlePara,
    Subtitle(u8),
    Poem,
    Stanza,
    Verse,
    Quote,
    Paragraph,
    TextAuthor,
    Code,
    Em,
    Strong,
    Strike,
    Sup,
    Sub,
    Link,
    Table,
    Row,
    Th,
    Td,
    Skip,
}

fn quick_xml_attr_local_ignore_case(
    e: &quick_xml::events::BytesStart<'_>,
    local: &[u8],
) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref().eq_ignore_ascii_case(local) {
            if let Ok(val) = attr.unescape_value() {
                return Some(val.into_owned());
            }
            return Some(String::from_utf8_lossy(&attr.value).into_owned());
        }
    }
    None
}

pub fn parse_fb2_to_html(xml: &str, fallback_title: &str) -> Result<(String, String), EngineError> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();

    let mut saw_root = false;
    let mut in_description = false;
    let mut in_title_info = false;
    let mut in_src_title_info = false;
    let mut in_book_title = false;
    let mut book_title_acc = String::new();
    let mut book_title: Option<String> = None;

    let mut body_out = String::new();
    let mut section_depth = 0usize;
    let mut skip_depth = 0usize;
    let mut title_depth = 0usize;
    let mut title_has_para = false;
    let mut stanza_has_v = false;
    let mut tag_stack: Vec<Fb2OpenTag> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let lname = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();

                if !saw_root {
                    if lname == "fictionbook" {
                        saw_root = true;
                        tag_stack.push(Fb2OpenTag::Skip);
                        buf.clear();
                        continue;
                    }
                    return Err(EngineError::ConversionFailed(format!(
                        "Invalid FB2 root element <{lname}>; expected <FictionBook>"
                    )));
                }
                if tag_stack.is_empty() {
                    return Err(EngineError::ConversionFailed(format!(
                        "Unexpected trailing XML element <{lname}> after </FictionBook>"
                    )));
                }

                if skip_depth > 0 {
                    if lname == "title-info" && in_description && !in_src_title_info {
                        in_title_info = true;
                    } else if lname == "src-title-info" && in_description {
                        in_src_title_info = true;
                    } else if lname == "book-title"
                        && in_description
                        && in_title_info
                        && !in_src_title_info
                    {
                        in_book_title = true;
                        book_title_acc.clear();
                    }
                    skip_depth += 1;
                    tag_stack.push(Fb2OpenTag::Skip);
                    buf.clear();
                    continue;
                }

                match lname.as_str() {
                    "description" | "binary" | "stylesheet" => {
                        if lname == "description" {
                            in_description = true;
                        }
                        skip_depth = 1;
                        tag_stack.push(Fb2OpenTag::Skip);
                    }
                    "section" => {
                        section_depth += 1;
                        body_out.push_str("<section>\n");
                        tag_stack.push(Fb2OpenTag::Section);
                    }
                    "title" => {
                        let lvl = section_depth.clamp(1, 6) as u8;
                        body_out.push_str(&format!("<h{lvl}>"));
                        title_depth += 1;
                        title_has_para = false;
                        tag_stack.push(Fb2OpenTag::Title(lvl));
                    }
                    "subtitle" => {
                        let lvl = (section_depth + 1).clamp(2, 6) as u8;
                        body_out.push_str(&format!("<h{lvl}>"));
                        title_depth += 1;
                        tag_stack.push(Fb2OpenTag::Subtitle(lvl));
                    }
                    "poem" => {
                        body_out.push_str("<div class=\"poem\">\n");
                        tag_stack.push(Fb2OpenTag::Poem);
                    }
                    "stanza" => {
                        body_out.push_str("<p class=\"stanza\">");
                        stanza_has_v = false;
                        tag_stack.push(Fb2OpenTag::Stanza);
                    }
                    "v" => {
                        if tag_stack.contains(&Fb2OpenTag::Stanza) {
                            if stanza_has_v {
                                body_out.push_str("<br/>");
                            }
                            stanza_has_v = true;
                            tag_stack.push(Fb2OpenTag::Verse);
                        } else {
                            body_out.push_str("<p>");
                            tag_stack.push(Fb2OpenTag::Paragraph);
                        }
                    }
                    "epigraph" | "cite" => {
                        body_out.push_str("<blockquote>\n");
                        tag_stack.push(Fb2OpenTag::Quote);
                    }
                    "text-author" => {
                        body_out.push_str("<p class=\"text-author\"><em>");
                        tag_stack.push(Fb2OpenTag::TextAuthor);
                    }
                    "p" => {
                        if title_depth > 0 {
                            if title_has_para {
                                body_out.push_str("<br/>");
                            }
                            title_has_para = true;
                            tag_stack.push(Fb2OpenTag::TitlePara);
                        } else {
                            body_out.push_str("<p>");
                            tag_stack.push(Fb2OpenTag::Paragraph);
                        }
                    }
                    "empty-line" => {
                        if title_depth > 0
                            || tag_stack.contains(&Fb2OpenTag::Paragraph)
                            || tag_stack.contains(&Fb2OpenTag::Stanza)
                        {
                            body_out.push_str("<br/>");
                        } else {
                            body_out.push_str("<p>&nbsp;</p>\n");
                        }
                        tag_stack.push(Fb2OpenTag::Skip);
                    }
                    "code" => {
                        body_out.push_str("<code>");
                        tag_stack.push(Fb2OpenTag::Code);
                    }
                    "emphasis" => {
                        body_out.push_str("<em>");
                        tag_stack.push(Fb2OpenTag::Em);
                    }
                    "strong" => {
                        body_out.push_str("<strong>");
                        tag_stack.push(Fb2OpenTag::Strong);
                    }
                    "strikethrough" => {
                        body_out.push_str("<del>");
                        tag_stack.push(Fb2OpenTag::Strike);
                    }
                    "sup" => {
                        body_out.push_str("<sup>");
                        tag_stack.push(Fb2OpenTag::Sup);
                    }
                    "sub" => {
                        body_out.push_str("<sub>");
                        tag_stack.push(Fb2OpenTag::Sub);
                    }
                    "a" => {
                        if let Some(href) = quick_xml_attr_local_ignore_case(e, b"href") {
                            let trimmed = href.trim();
                            if is_safe_xhtml_url(trimmed, false) {
                                body_out.push_str(&format!("<a href=\"{}\">", html_escape(trimmed)));
                                tag_stack.push(Fb2OpenTag::Link);
                            } else {
                                tag_stack.push(Fb2OpenTag::Skip);
                            }
                        } else {
                            tag_stack.push(Fb2OpenTag::Skip);
                        }
                    }
                    "table" => {
                        body_out.push_str("<table>\n");
                        tag_stack.push(Fb2OpenTag::Table);
                    }
                    "tr" => {
                        body_out.push_str("<tr>");
                        tag_stack.push(Fb2OpenTag::Row);
                    }
                    "th" => {
                        body_out.push_str("<th>");
                        tag_stack.push(Fb2OpenTag::Th);
                    }
                    "td" => {
                        body_out.push_str("<td>");
                        tag_stack.push(Fb2OpenTag::Td);
                    }
                    _ => {
                        tag_stack.push(Fb2OpenTag::Skip);
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let lname = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                if !saw_root {
                    if lname == "fictionbook" {
                        saw_root = true;
                        buf.clear();
                        continue;
                    }
                    return Err(EngineError::ConversionFailed(format!(
                        "Invalid FB2 root element <{lname}>; expected <FictionBook>"
                    )));
                }
                if tag_stack.is_empty() {
                    return Err(EngineError::ConversionFailed(format!(
                        "Unexpected trailing XML element <{lname}/> after </FictionBook>"
                    )));
                }
                if skip_depth == 0 && lname == "empty-line" {
                    if title_depth > 0
                        || tag_stack.contains(&Fb2OpenTag::Paragraph)
                        || tag_stack.contains(&Fb2OpenTag::Stanza)
                    {
                        body_out.push_str("<br/>");
                    } else {
                        body_out.push_str("<p>&nbsp;</p>\n");
                    }
                } else if skip_depth == 0 && lname == "p" && title_depth == 0 {
                    body_out.push_str("<p>&nbsp;</p>\n");
                }
            }
            Ok(Event::Text(ref t)) => {
                let text = match t.unescape() {
                    Ok(decoded) => decoded.into_owned(),
                    Err(_) => decode_html_entities(&String::from_utf8_lossy(t.as_ref())),
                };
                if in_book_title {
                    book_title_acc.push_str(&text);
                } else if skip_depth == 0 && saw_root && !tag_stack.is_empty() {
                    let in_mixed_content = matches!(
                        tag_stack.last(),
                        Some(
                            Fb2OpenTag::Paragraph
                                | Fb2OpenTag::Title(_)
                                | Fb2OpenTag::TitlePara
                                | Fb2OpenTag::Subtitle(_)
                                | Fb2OpenTag::Verse
                                | Fb2OpenTag::TextAuthor
                                | Fb2OpenTag::Code
                                | Fb2OpenTag::Em
                                | Fb2OpenTag::Strong
                                | Fb2OpenTag::Strike
                                | Fb2OpenTag::Sup
                                | Fb2OpenTag::Sub
                                | Fb2OpenTag::Link
                                | Fb2OpenTag::Th
                                | Fb2OpenTag::Td
                        )
                    );
                    if in_mixed_content || !text.trim().is_empty() {
                        body_out.push_str(&html_escape(&text));
                    }
                }
            }
            Ok(Event::CData(ref t)) => {
                let text = String::from_utf8_lossy(t.as_ref());
                if in_book_title {
                    book_title_acc.push_str(&text);
                } else if skip_depth == 0 && saw_root && !tag_stack.is_empty() {
                    body_out.push_str(&html_escape(&text));
                }
            }
            Ok(Event::End(ref e)) => {
                let lname = String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase();
                if skip_depth > 0 {
                    if lname == "book-title" && in_book_title {
                        in_book_title = false;
                        let candidate = book_title_acc.trim();
                        if !candidate.is_empty() && book_title.is_none() {
                            book_title = Some(candidate.to_string());
                        }
                    } else if lname == "title-info" && in_description && !in_src_title_info {
                        in_title_info = false;
                    } else if lname == "src-title-info" && in_description {
                        in_src_title_info = false;
                    } else if lname == "description" {
                        in_description = false;
                    }
                    skip_depth -= 1;
                    tag_stack.pop();
                    buf.clear();
                    continue;
                }

                if let Some(open) = tag_stack.pop() {
                    match open {
                        Fb2OpenTag::Section => {
                            section_depth = section_depth.saturating_sub(1);
                            body_out.push_str("</section>\n");
                        }
                        Fb2OpenTag::Title(lvl) => {
                            title_depth = title_depth.saturating_sub(1);
                            body_out.push_str(&format!("</h{lvl}>\n"));
                        }
                        Fb2OpenTag::Subtitle(lvl) => {
                            title_depth = title_depth.saturating_sub(1);
                            body_out.push_str(&format!("</h{lvl}>\n"));
                        }
                        Fb2OpenTag::Poem => body_out.push_str("</div>\n"),
                        Fb2OpenTag::Stanza => body_out.push_str("</p>\n"),
                        Fb2OpenTag::Verse | Fb2OpenTag::TitlePara | Fb2OpenTag::Skip => {}
                        Fb2OpenTag::Quote => body_out.push_str("</blockquote>\n"),
                        Fb2OpenTag::Paragraph => body_out.push_str("</p>\n"),
                        Fb2OpenTag::TextAuthor => body_out.push_str("</em></p>\n"),
                        Fb2OpenTag::Code => body_out.push_str("</code>"),
                        Fb2OpenTag::Em => body_out.push_str("</em>"),
                        Fb2OpenTag::Strong => body_out.push_str("</strong>"),
                        Fb2OpenTag::Strike => body_out.push_str("</del>"),
                        Fb2OpenTag::Sup => body_out.push_str("</sup>"),
                        Fb2OpenTag::Sub => body_out.push_str("</sub>"),
                        Fb2OpenTag::Link => body_out.push_str("</a>"),
                        Fb2OpenTag::Table => body_out.push_str("</table>\n"),
                        Fb2OpenTag::Row => body_out.push_str("</tr>\n"),
                        Fb2OpenTag::Th => body_out.push_str("</th>"),
                        Fb2OpenTag::Td => body_out.push_str("</td>"),
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(EngineError::ConversionFailed(format!(
                    "Malformed FB2 XML: {e}"
                )));
            }
        }
        buf.clear();
    }

    if !saw_root {
        return Err(EngineError::ConversionFailed(
            "Empty or invalid FB2 XML: missing <FictionBook>".to_string(),
        ));
    }
    if !tag_stack.is_empty() {
        return Err(EngineError::ConversionFailed(
            "Malformed FB2 XML: unexpected EOF with unclosed elements".to_string(),
        ));
    }

    let final_title = book_title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            if fallback_title.trim().is_empty() {
                "FictionBook Document".to_string()
            } else {
                fallback_title.trim().to_string()
            }
        });
    let full_html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:11.5pt;line-height:1.5;color:#111;}}h1,h2,h3{{margin:14pt 0 6pt 0;}}pre,code{{font-family:Menlo,monospace;font-size:10pt;background:#f4f4f6;}}blockquote{{border-left:3px solid #bbb;padding-left:10pt;color:#444;}}p{{margin:0 0 6pt 0;}}p.stanza{{white-space:pre-line;}}</style>\
        </head><body>\n{body_out}</body></html>",
        html_escape(&final_title)
    );
    Ok((final_title, full_html))
}

pub fn read_fb2_to_html(path: &Path) -> Result<(String, String), EngineError> {
    let raw = read_text_bounded_with_utf16(path, MAX_HTML_READ_BYTES, "FB2")?;
    let fallback = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("FictionBook Document");
    parse_fb2_to_html(&raw, fallback)
}

#[derive(Clone, PartialEq, Eq)]
enum Fb2WriteInline {
    Strong,
    Emphasis,
    Code,
    Strikethrough,
    Sup,
    Sub,
    Link(String),
}

impl Fb2WriteInline {
    fn open_tag(&self) -> String {
        match self {
            Self::Strong => "<strong>".to_string(),
            Self::Emphasis => "<emphasis>".to_string(),
            Self::Code => "<code>".to_string(),
            Self::Strikethrough => "<strikethrough>".to_string(),
            Self::Sup => "<sup>".to_string(),
            Self::Sub => "<sub>".to_string(),
            Self::Link(href) => format!("<a l:href=\"{}\">", xml_escape(href)),
        }
    }

    fn close_tag(&self) -> &'static str {
        match self {
            Self::Strong => "</strong>",
            Self::Emphasis => "</emphasis>",
            Self::Code => "</code>",
            Self::Strikethrough => "</strikethrough>",
            Self::Sup => "</sup>",
            Self::Sub => "</sub>",
            Self::Link(_) => "</a>",
        }
    }

    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

pub fn write_fb2_from_html(title: &str, html_input: &str) -> String {
    let doc_title = extract_html_title(html_input)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            if title.trim().is_empty() {
                "FLIP Document".to_string()
            } else {
                title.trim().to_string()
            }
        });
    let body_slice = extract_html_body_slice(html_input);

    let mut sections_xml = String::new();
    let mut section_open = false;
    let mut section_has_body = false;

    enum BlockTarget {
        None,
        SectionTitle,
        Subtitle,
        Para,
        TableCell(&'static str),
    }

    let mut block_target = BlockTarget::None;
    let mut block_buf = String::new();
    let mut block_has_non_ws = false;
    let mut inline_stack: Vec<Fb2WriteInline> = Vec::new();
    let mut ignored_link_depth = 0usize;
    let mut in_pre = false;
    let mut pre_buf = String::new();
    let mut cite_depth = 0usize;
    let mut in_table = false;
    let mut table_nest_depth = 0usize;
    let mut in_tr = false;
    let mut list_stack: Vec<(bool, usize)> = Vec::new();
    let mut pending_li_prefix: Option<String> = None;

    let close_all_inline = |buf: &mut String, stack: &mut Vec<Fb2WriteInline>| {
        while let Some(tag) = stack.pop() {
            buf.push_str(tag.close_tag());
        }
    };

    let close_inline_until =
        |kind: &Fb2WriteInline, buf: &mut String, stack: &mut Vec<Fb2WriteInline>| {
            if let Some(pos) = stack.iter().rposition(|k| k.same_kind(kind)) {
                while stack.len() > pos {
                    if let Some(popped) = stack.pop() {
                        buf.push_str(popped.close_tag());
                    }
                }
            }
        };

    let ensure_section = |out: &mut String, open: &mut bool| {
        if !*open {
            out.push_str("    <section>\n");
            *open = true;
        }
    };

    let finish_block = |out: &mut String,
                        sec_open: &mut bool,
                        sec_body: &mut bool,
                        target: &mut BlockTarget,
                        buf: &mut String,
                        has_non_ws: &mut bool,
                        stack: &mut Vec<Fb2WriteInline>,
                        prefix: &mut Option<String>,
                        ignored_links: &mut usize| {
        close_all_inline(buf, stack);
        *ignored_links = 0;
        match *target {
            BlockTarget::None => {}
            BlockTarget::SectionTitle => {
                if *has_non_ws {
                    ensure_section(out, sec_open);
                    out.push_str("      <title><p>");
                    out.push_str(buf);
                    out.push_str("</p></title>\n");
                }
            }
            BlockTarget::Subtitle => {
                if *has_non_ws {
                    ensure_section(out, sec_open);
                    out.push_str("      <subtitle>");
                    out.push_str(buf);
                    out.push_str("</subtitle>\n");
                    *sec_body = true;
                }
            }
            BlockTarget::Para => {
                ensure_section(out, sec_open);
                if *has_non_ws {
                    out.push_str("      <p>");
                    out.push_str(buf);
                    out.push_str("</p>\n");
                } else {
                    out.push_str("      <empty-line/>\n");
                }
                *sec_body = true;
            }
            BlockTarget::TableCell(cell_tag) => {
                out.push('<');
                out.push_str(cell_tag);
                out.push('>');
                out.push_str(buf);
                out.push_str("</");
                out.push_str(cell_tag);
                out.push('>');
            }
        }
        *target = BlockTarget::None;
        buf.clear();
        *has_non_ws = false;
        *prefix = None;
    };

    let flush_pre =
        |out: &mut String, sec_open: &mut bool, sec_body: &mut bool, pre: &mut String| {
            ensure_section(out, sec_open);
            let decoded = decode_html_entities(pre);
            let normalized = decoded.replace("\r\n", "\n").replace('\r', "\n");
            let trimmed_end = normalized.strip_suffix('\n').unwrap_or(&normalized);
            for line in trimmed_end.split('\n') {
                if line.trim().is_empty() {
                    out.push_str("      <empty-line/>\n");
                } else {
                    out.push_str("      <p><code>");
                    out.push_str(&xml_escape_text(line));
                    out.push_str("</code></p>\n");
                }
            }
            *sec_body = true;
            pre.clear();
        };

    let bytes = body_slice.as_bytes();
    let mut i = 0;
    let mut skip_tag: Option<(String, usize)> = None;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if body_slice[i..].starts_with("<!--") {
                if let Some(end_rel) = body_slice[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                } else {
                    break;
                }
                continue;
            }
            if body_slice[i..].starts_with("<![CDATA[") {
                let (cdata_end, next_i) = if let Some(end_rel) = body_slice[i + 9..].find("]]>") {
                    (i + 9 + end_rel, i + 9 + end_rel + 3)
                } else {
                    (bytes.len(), bytes.len())
                };
                if skip_tag.is_none() {
                    let raw = &body_slice[i + 9..cdata_end];
                    if in_pre {
                        pre_buf.push_str(raw);
                    } else {
                        if matches!(block_target, BlockTarget::None)
                            && !in_table
                            && !raw.trim().is_empty()
                        {
                            block_target = BlockTarget::Para;
                        }
                        if !matches!(block_target, BlockTarget::None) {
                            if raw.chars().any(|c| !c.is_whitespace()) {
                                block_has_non_ws = true;
                            }
                            block_buf.push_str(&xml_escape_text(raw));
                        }
                    }
                }
                i = next_i;
                continue;
            }
            if !matches!(
                bytes.get(i + 1),
                Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')
            ) {
                if skip_tag.is_none() {
                    if in_pre {
                        pre_buf.push('<');
                    } else if !in_table || matches!(block_target, BlockTarget::TableCell(_)) {
                        if matches!(block_target, BlockTarget::None) {
                            block_target = BlockTarget::Para;
                        }
                        block_has_non_ws = true;
                        block_buf.push_str("&lt;");
                    }
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
                break;
            }

            let raw_tag = body_slice[i + 1..j].trim();
            i = j + 1;
            if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }

            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let self_closing = !is_close && has_xml_self_closing_slash(inner);
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if !is_close
                && matches!(tag_name.as_str(), "script" | "style" | "noscript" | "template")
                && !self_closing
            {
                i = find_raw_close_tag(body_slice, i, &tag_name).unwrap_or(bytes.len());
                continue;
            }
            if let Some((ref skipping, ref mut depth)) = skip_tag {
                if &tag_name == skipping {
                    if is_close {
                        *depth -= 1;
                        if *depth == 0 {
                            skip_tag = None;
                        }
                    } else if !self_closing {
                        *depth += 1;
                    }
                }
                continue;
            }
            if !is_close
                && matches!(tag_name.as_str(), "head" | "svg" | "iframe" | "object" | "embed")
                && !self_closing
            {
                skip_tag = Some((tag_name, 1));
                continue;
            }

            if in_pre {
                if is_close && tag_name == "pre" {
                    if matches!(block_target, BlockTarget::TableCell(_)) {
                        let decoded = decode_html_entities(&pre_buf);
                        let normalized = decoded.replace("\r\n", "\n").replace('\r', "\n");
                        let trimmed_end = normalized.strip_suffix('\n').unwrap_or(&normalized);
                        for line in trimmed_end.split('\n') {
                            let piece = line.trim();
                            if !piece.is_empty() {
                                if block_has_non_ws
                                    && !block_buf.is_empty()
                                    && !block_buf.ends_with(' ')
                                {
                                    block_buf.push(' ');
                                }
                                block_buf.push_str("<code>");
                                block_buf.push_str(&xml_escape_text(piece));
                                block_buf.push_str("</code>");
                                block_has_non_ws = true;
                            }
                        }
                        pre_buf.clear();
                    } else if !in_table {
                        flush_pre(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut pre_buf,
                        );
                    } else {
                        pre_buf.clear();
                    }
                    in_pre = false;
                } else if !is_close && tag_name == "br" {
                    pre_buf.push('\n');
                }
                continue;
            }

            let in_cell = matches!(block_target, BlockTarget::TableCell(_));

            match (is_close, tag_name.as_str()) {
                (false, "h1" | "h2") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        if in_tr {
                            sections_xml.push_str("</tr>\n");
                            in_tr = false;
                        }
                        if in_table {
                            sections_xml.push_str("      </table>\n");
                            in_table = false;
                            table_nest_depth = 0;
                        }
                        if cite_depth > 0 {
                            sections_xml.push_str("      </cite>\n");
                            cite_depth = 0;
                        }
                        if section_open && section_has_body {
                            sections_xml.push_str("    </section>\n");
                            section_open = false;
                            section_has_body = false;
                        } else if section_open {
                            sections_xml.push_str("      <empty-line/>\n    </section>\n");
                            section_open = false;
                        }
                        block_target = BlockTarget::SectionTitle;
                    }
                }
                (true, "h1" | "h2") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if matches!(block_target, BlockTarget::SectionTitle) {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                }
                (false, "h3" | "h4" | "h5" | "h6") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::Subtitle;
                    }
                }
                (true, "h3" | "h4" | "h5" | "h6") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if matches!(block_target, BlockTarget::Subtitle) {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                }
                (false, "pre") => {
                    if !in_cell && !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    } else if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                        block_buf.push(' ');
                    }
                    in_pre = true;
                    pre_buf.clear();
                }
                (false, "blockquote") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        ensure_section(&mut sections_xml, &mut section_open);
                        if cite_depth == 0 {
                            sections_xml.push_str("      <cite>\n");
                            section_has_body = true;
                        }
                        cite_depth += 1;
                    }
                }
                (true, "blockquote") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        if cite_depth > 0 {
                            cite_depth -= 1;
                            if cite_depth == 0 {
                                sections_xml.push_str("      </cite>\n");
                            }
                        }
                    }
                }
                (false, "ul") => {
                    if !in_cell && !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                    list_stack.push((false, 0));
                }
                (false, "ol") => {
                    if !in_cell && !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                    list_stack.push((true, 0));
                }
                (true, "ul" | "ol") => {
                    if !in_cell && !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                    list_stack.pop();
                }
                (false, "li") => {
                    let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                    let marker = if let Some((true, count)) = list_stack.last_mut() {
                        *count += 1;
                        format!("{indent}{count}. ")
                    } else {
                        format!("{indent}- ")
                    };
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push_str("; ");
                        }
                        block_buf.push_str(&xml_escape_text(&marker));
                        block_has_non_ws = true;
                        pending_li_prefix = Some(marker);
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::Para;
                        block_buf.push_str(&xml_escape_text(&marker));
                        block_has_non_ws = true;
                        pending_li_prefix = Some(marker);
                    }
                }
                (true, "li") => {
                    if !in_cell && !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    } else if in_cell {
                        pending_li_prefix = None;
                    }
                }
                (false, "p" | "div") => {
                    if in_cell {
                        if block_has_non_ws
                            && pending_li_prefix.is_none()
                            && !block_buf.ends_with(' ')
                        {
                            block_buf.push(' ');
                        }
                    } else if !in_table
                        && !(matches!(block_target, BlockTarget::Para)
                            && pending_li_prefix.is_some())
                    {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::Para;
                    }
                }
                (true, "p" | "div") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                }
                (false, "br") => {
                    if matches!(block_target, BlockTarget::Para) {
                        let saved_stack = inline_stack.clone();
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::Para;
                        for tag in saved_stack {
                            block_buf.push_str(&tag.open_tag());
                            inline_stack.push(tag);
                        }
                    } else if matches!(
                        block_target,
                        BlockTarget::SectionTitle
                            | BlockTarget::Subtitle
                            | BlockTarget::TableCell(_)
                    ) && !block_buf.ends_with(' ')
                    {
                        block_buf.push(' ');
                    }
                }
                (false, "hr") => {
                    if in_cell {
                        if block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if !in_table {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        ensure_section(&mut sections_xml, &mut section_open);
                        sections_xml.push_str("      <p>---</p>\n");
                        section_has_body = true;
                    }
                }
                (false, "table") => {
                    if in_table {
                        table_nest_depth += 1;
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        ensure_section(&mut sections_xml, &mut section_open);
                        sections_xml.push_str("      <table>\n");
                        in_table = true;
                        table_nest_depth = 0;
                        section_has_body = true;
                    }
                }
                (true, "table") => {
                    if table_nest_depth > 0 {
                        table_nest_depth -= 1;
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        if in_tr {
                            sections_xml.push_str("</tr>\n");
                            in_tr = false;
                        }
                        if in_table {
                            sections_xml.push_str("      </table>\n");
                            in_table = false;
                        }
                    }
                }
                (false, "tr") if in_table => {
                    if table_nest_depth > 0 {
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        if in_tr {
                            sections_xml.push_str("</tr>\n");
                        }
                        sections_xml.push_str("        <tr>");
                        in_tr = true;
                    }
                }
                (true, "tr") if in_table => {
                    if table_nest_depth > 0 {
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push(' ');
                        }
                    } else if in_tr {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        sections_xml.push_str("</tr>\n");
                        in_tr = false;
                    }
                }
                (false, "th") if in_table && in_tr => {
                    if table_nest_depth > 0 {
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push_str(" | ");
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::TableCell("th");
                    }
                }
                (false, "td") if in_table && in_tr => {
                    if table_nest_depth > 0 {
                        if in_cell && block_has_non_ws && !block_buf.ends_with(' ') {
                            block_buf.push_str(" | ");
                        }
                    } else {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                        block_target = BlockTarget::TableCell("td");
                    }
                }
                (true, "th" | "td") if in_table && in_tr => {
                    if table_nest_depth == 0 && in_cell {
                        finish_block(
                            &mut sections_xml,
                            &mut section_open,
                            &mut section_has_body,
                            &mut block_target,
                            &mut block_buf,
                            &mut block_has_non_ws,
                            &mut inline_stack,
                            &mut pending_li_prefix,
                            &mut ignored_link_depth,
                        );
                    }
                }
                (false, "strong" | "b") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Strong.open_tag());
                    inline_stack.push(Fb2WriteInline::Strong);
                }
                (true, "strong" | "b") if !in_table || in_cell => close_inline_until(
                    &Fb2WriteInline::Strong,
                    &mut block_buf,
                    &mut inline_stack,
                ),
                (false, "em" | "i") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Emphasis.open_tag());
                    inline_stack.push(Fb2WriteInline::Emphasis);
                }
                (true, "em" | "i") if !in_table || in_cell => close_inline_until(
                    &Fb2WriteInline::Emphasis,
                    &mut block_buf,
                    &mut inline_stack,
                ),
                (false, "code" | "kbd" | "samp" | "tt") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Code.open_tag());
                    inline_stack.push(Fb2WriteInline::Code);
                }
                (true, "code" | "kbd" | "samp" | "tt") if !in_table || in_cell => {
                    close_inline_until(&Fb2WriteInline::Code, &mut block_buf, &mut inline_stack)
                }
                (false, "del" | "s" | "strike") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Strikethrough.open_tag());
                    inline_stack.push(Fb2WriteInline::Strikethrough);
                }
                (true, "del" | "s" | "strike") if !in_table || in_cell => close_inline_until(
                    &Fb2WriteInline::Strikethrough,
                    &mut block_buf,
                    &mut inline_stack,
                ),
                (false, "sup") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Sup.open_tag());
                    inline_stack.push(Fb2WriteInline::Sup);
                }
                (true, "sup") if !in_table || in_cell => {
                    close_inline_until(&Fb2WriteInline::Sup, &mut block_buf, &mut inline_stack)
                }
                (false, "sub") if !in_table || in_cell => {
                    if matches!(block_target, BlockTarget::None) {
                        block_target = BlockTarget::Para;
                    }
                    block_buf.push_str(&Fb2WriteInline::Sub.open_tag());
                    inline_stack.push(Fb2WriteInline::Sub);
                }
                (true, "sub") if !in_table || in_cell => {
                    close_inline_until(&Fb2WriteInline::Sub, &mut block_buf, &mut inline_stack)
                }
                (false, "a") if !in_table || in_cell => {
                    if let Some(href) = extract_attr(inner, "href") {
                        let decoded_href = decode_html_entities(&href);
                        let trimmed = decoded_href.trim();
                        if is_safe_xhtml_url(trimmed, false) {
                            if inline_stack
                                .iter()
                                .any(|k| matches!(k, Fb2WriteInline::Link(_)))
                            {
                                close_inline_until(
                                    &Fb2WriteInline::Link(String::new()),
                                    &mut block_buf,
                                    &mut inline_stack,
                                );
                            }
                            if matches!(block_target, BlockTarget::None) {
                                block_target = BlockTarget::Para;
                            }
                            let link = Fb2WriteInline::Link(trimmed.to_string());
                            block_buf.push_str(&link.open_tag());
                            inline_stack.push(link);
                        } else {
                            ignored_link_depth += 1;
                        }
                    } else {
                        ignored_link_depth += 1;
                    }
                }
                (true, "a") if !in_table || in_cell => {
                    if ignored_link_depth > 0 {
                        ignored_link_depth -= 1;
                    } else {
                        close_inline_until(
                            &Fb2WriteInline::Link(String::new()),
                            &mut block_buf,
                            &mut inline_stack,
                        );
                    }
                }
                _ => {}
            }
            continue;
        }

        let next_tag_rel = body_slice[i..].find('<').unwrap_or(body_slice.len() - i);
        let raw_chunk = &body_slice[i..i + next_tag_rel];
        i += next_tag_rel;
        if skip_tag.is_some() {
            continue;
        }
        if in_pre {
            pre_buf.push_str(raw_chunk);
            continue;
        }
        let decoded = decode_html_entities_inner(raw_chunk, true);
        let has_visible = decoded
            .chars()
            .any(|c| !c.is_whitespace() && c != '\u{00A0}');
        let collapsed: String = decoded
            .chars()
            .map(|c| {
                if c == '\r' || c == '\n' || c == '\u{00A0}' {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        if matches!(block_target, BlockTarget::None) && !in_table && !collapsed.trim().is_empty() {
            block_target = BlockTarget::Para;
        }
        if !matches!(block_target, BlockTarget::None) {
            if has_visible {
                block_has_non_ws = true;
            }
            block_buf.push_str(&xml_escape_text(&collapsed));
        }
    }

    if in_pre && !in_table {
        flush_pre(
            &mut sections_xml,
            &mut section_open,
            &mut section_has_body,
            &mut pre_buf,
        );
    }
    finish_block(
        &mut sections_xml,
        &mut section_open,
        &mut section_has_body,
        &mut block_target,
        &mut block_buf,
        &mut block_has_non_ws,
        &mut inline_stack,
        &mut pending_li_prefix,
        &mut ignored_link_depth,
    );
    if in_tr {
        sections_xml.push_str("</tr>\n");
    }
    if in_table {
        sections_xml.push_str("      </table>\n");
    }
    if cite_depth > 0 {
        sections_xml.push_str("      </cite>\n");
    }
    if section_open {
        if !section_has_body {
            sections_xml.push_str("      <empty-line/>\n");
        }
        sections_xml.push_str("    </section>\n");
    } else {
        sections_xml.push_str("    <section><empty-line/></section>\n");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\" xmlns:l=\"http://www.w3.org/1999/xlink\">\n\
        \x20 <description>\n\
        \x20   <title-info>\n\
        \x20     <genre>nonfiction</genre>\n\
        \x20     <author><first-name>FLIP</first-name><last-name>Document</last-name></author>\n\
        \x20     <book-title>{}</book-title>\n\
        \x20     <lang>en</lang>\n\
        \x20   </title-info>\n\
        \x20 </description>\n\
        \x20 <body>\n\
        {sections_xml}\
        \x20 </body>\n\
        </FictionBook>\n",
        xml_escape_text(&doc_title)
    )
}

// =========================================================================
// Native Pure-Rust reStructuredText (RST) Reader & Writer
// =========================================================================

fn rst_adornment_char(trimmed: &str) -> Option<char> {
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if !matches!(first, '=' | '-' | '~' | '^' | '"' | '#' | '*' | '+' | '_') {
        return None;
    }
    if chars.all(|c| c == first) {
        Some(first)
    } else {
        None
    }
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn detect_rst_list_marker(line: &str) -> Option<(usize, bool, usize)> {
    let indent = leading_spaces(line);
    let rest = &line[indent..];
    if rest.len() >= 2
        && matches!(rest.as_bytes()[0], b'-' | b'*' | b'+')
        && rest.as_bytes()[1] == b' '
    {
        return Some((indent, false, indent + 2));
    }
    if rest.starts_with("#. ") {
        return Some((indent, true, indent + 3));
    }
    if rest.starts_with('(') {
        if let Some(close_pos) = rest.find(')') {
            let inside = &rest[1..close_pos];
            if !inside.is_empty()
                && inside.chars().all(|c| c.is_ascii_digit())
                && rest.as_bytes().get(close_pos + 1) == Some(&b' ')
            {
                return Some((indent, true, indent + close_pos + 2));
            }
        }
    }
    if let Some(dot_pos) = rest.find(". ") {
        let num = &rest[..dot_pos];
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return Some((indent, true, indent + dot_pos + 2));
        }
    }
    None
}

fn normalize_rst_ref_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn render_rst_inline(input: &str, targets: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '\\' && i + 1 < len {
            out.push_str(&html_escape(&chars[i + 1].to_string()));
            i += 2;
            continue;
        }

        if chars[i] == '`' && i + 1 < len && chars[i + 1] == '`' {
            let mut j = i + 2;
            let mut found = None;
            while j + 1 < len {
                if chars[j] == '`' && chars[j + 1] == '`' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                let code: String = chars[i + 2..end].iter().collect();
                out.push_str("<code>");
                out.push_str(&html_escape(&code));
                out.push_str("</code>");
                i = end + 2;
                continue;
            }
        }

        if chars[i] == ':' {
            let slice: String = chars[i..].iter().collect();
            let mut matched_role = false;
            for (prefix, tag) in [
                (":code:`", "code"),
                (":literal:`", "code"),
                (":strong:`", "strong"),
                (":emphasis:`", "em"),
            ] {
                if let Some(after) = slice.strip_prefix(prefix) {
                    if let Some(end_bt) = after.find('`') {
                        let inner = &after[..end_bt];
                        out.push_str(&format!("<{tag}>{}</{tag}>", html_escape(inner)));
                        i += prefix.chars().count() + inner.chars().count() + 1;
                        matched_role = true;
                        break;
                    }
                }
            }
            if matched_role {
                continue;
            }
        }

        if chars[i] == '`' {
            let prev_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            if prev_ok {
                let mut j = i + 1;
                let mut found_end = None;
                while j < len {
                    if chars[j] == '\\' && j + 1 < len {
                        j += 2;
                        continue;
                    }
                    if chars[j] == '`' {
                        found_end = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end_bt) = found_end {
                    let has_double_under =
                        end_bt + 2 < len && chars[end_bt + 1] == '_' && chars[end_bt + 2] == '_';
                    let has_single_under =
                        !has_double_under && end_bt + 1 < len && chars[end_bt + 1] == '_';
                    if has_double_under || has_single_under {
                        let raw_inner: String = chars[i + 1..end_bt].iter().collect();
                        let advance = if has_double_under {
                            end_bt + 3
                        } else {
                            end_bt + 2
                        };
                        if let (Some(lt), true) =
                            (raw_inner.rfind('<'), raw_inner.trim_end().ends_with('>'))
                        {
                            let label = raw_inner[..lt].trim();
                            let url_part = raw_inner[lt + 1..].trim_end();
                            let url = url_part.strip_suffix('>').unwrap_or(url_part).trim();
                            let display = if label.is_empty() { url } else { label };
                            if is_safe_xhtml_url(url, false) {
                                out.push_str(&format!(
                                    "<a href=\"{}\">{}</a>",
                                    html_escape(url),
                                    html_escape(display)
                                ));
                            } else {
                                out.push_str(&html_escape(display));
                            }
                            i = advance;
                            continue;
                        } else {
                            let key = normalize_rst_ref_key(&raw_inner);
                            if let Some(url) = targets.get(&key) {
                                if is_safe_xhtml_url(url, false) {
                                    out.push_str(&format!(
                                        "<a href=\"{}\">{}</a>",
                                        html_escape(url),
                                        html_escape(raw_inner.trim())
                                    ));
                                    i = advance;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        if chars[i] == '*' && i + 1 < len && chars[i + 1] == '*' {
            let has_inner_ws = i + 2 >= len || chars[i + 2].is_whitespace();
            if !has_inner_ws {
                let mut j = i + 2;
                let mut found = None;
                while j + 1 < len {
                    if chars[j] == '\\' && j + 1 < len {
                        j += 2;
                        continue;
                    }
                    if chars[j] == '*'
                        && chars[j + 1] == '*'
                        && j > i + 2
                        && !chars[j - 1].is_whitespace()
                    {
                        found = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = found {
                    let inner: String = chars[i + 2..end].iter().collect();
                    out.push_str("<strong>");
                    out.push_str(&render_rst_inline(&inner, targets));
                    out.push_str("</strong>");
                    i = end + 2;
                    continue;
                }
            }
        }

        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            let prev_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            let has_inner_ws = i + 1 >= len || chars[i + 1].is_whitespace();
            if prev_ok && !has_inner_ws {
                let mut j = i + 1;
                let mut found = None;
                while j < len {
                    if chars[j] == '\\' && j + 1 < len {
                        j += 2;
                        continue;
                    }
                    if chars[j] == '*' && j > i + 1 && !chars[j - 1].is_whitespace() {
                        found = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = found {
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push_str("<em>");
                    out.push_str(&html_escape(&inner));
                    out.push_str("</em>");
                    i = end + 1;
                    continue;
                }
            }
        }

        out.push_str(&html_escape(&chars[i].to_string()));
        i += 1;
    }
    out
}

fn consume_indented_literal_block(lines: &[&str], idx: &mut usize) -> Option<String> {
    let mut look = *idx;
    while look < lines.len() && lines[look].trim().is_empty() {
        look += 1;
    }
    if look >= lines.len() {
        return None;
    }
    let first_indent = leading_spaces(lines[look]);
    if first_indent == 0 && !lines[look].starts_with('>') {
        return None;
    }

    if first_indent == 0 && lines[look].starts_with('>') {
        let mut block = Vec::new();
        let mut cur = look;
        while cur < lines.len() {
            let l = lines[cur];
            if l.trim().is_empty() || l.starts_with('>') {
                block.push(l);
                cur += 1;
            } else {
                break;
            }
        }
        *idx = cur;
        return Some(block.join("\n").trim_end_matches('\n').to_string());
    }

    let mut raw_lines = Vec::new();
    let mut cur = look;
    let mut min_indent = usize::MAX;
    while cur < lines.len() {
        let line = lines[cur];
        if line.trim().is_empty() {
            let mut next_nb = cur + 1;
            while next_nb < lines.len() && lines[next_nb].trim().is_empty() {
                next_nb += 1;
            }
            if next_nb < lines.len() && leading_spaces(lines[next_nb]) > 0 {
                raw_lines.push("");
                cur += 1;
                continue;
            } else {
                break;
            }
        }
        let ind = leading_spaces(line);
        if ind == 0 {
            break;
        }
        min_indent = min_indent.min(ind);
        raw_lines.push(line);
        cur += 1;
    }

    *idx = cur;
    if raw_lines.is_empty() {
        return None;
    }
    let strip = if min_indent == usize::MAX { 0 } else { min_indent };
    let dedented: Vec<String> = raw_lines
        .into_iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l.chars().skip(strip).collect()
            }
        })
        .collect();
    Some(dedented.join("\n").trim_end_matches('\n').to_string())
}

fn split_paragraph_double_colon(para_lines: &[&str]) -> (String, bool) {
    let mut lines: Vec<String> = para_lines.iter().map(|s| s.trim_end().to_string()).collect();
    if let Some(last) = lines.last_mut() {
        let trimmed = last.trim();
        if trimmed == "::" {
            lines.pop();
            let text = lines
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            return (text, true);
        }
        if last.ends_with("::") && !last.ends_with("\\::") {
            let without = &last[..last.len() - 2];
            if without
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
            {
                *last = without.trim_end().to_string();
            } else {
                *last = format!("{without}:");
            }
            let text = lines
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            return (text, true);
        }
    }
    let text = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (text, false)
}

fn parse_rst_list(
    lines: &[&str],
    idx: &mut usize,
    base_indent: usize,
    targets: &HashMap<String, String>,
    out: &mut String,
) {
    let Some((_, ordered, _)) = lines.get(*idx).and_then(|l| detect_rst_list_marker(l)) else {
        return;
    };
    out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });

    while *idx < lines.len() {
        if lines[*idx].trim().is_empty() {
            let mut next_nb = *idx + 1;
            while next_nb < lines.len() && lines[next_nb].trim().is_empty() {
                next_nb += 1;
            }
            if let Some((ni, n_ord, _)) = lines.get(next_nb).and_then(|l| detect_rst_list_marker(l))
            {
                if ni == base_indent && n_ord == ordered {
                    *idx = next_nb;
                    continue;
                }
            }
            break;
        }

        let Some((indent, is_ord, content_col)) = detect_rst_list_marker(lines[*idx]) else {
            break;
        };
        if indent != base_indent || is_ord != ordered {
            break;
        }

        out.push_str("<li>");
        let first_text = lines[*idx][content_col..].trim_end();
        let mut item_para: Vec<&str> = vec![first_text];
        *idx += 1;

        while *idx < lines.len() {
            let line = lines[*idx];
            if line.trim().is_empty() {
                let mut next_nb = *idx + 1;
                while next_nb < lines.len() && lines[next_nb].trim().is_empty() {
                    next_nb += 1;
                }
                if next_nb >= lines.len() {
                    break;
                }
                if let Some((ci, _, _)) = detect_rst_list_marker(lines[next_nb]) {
                    if ci > base_indent {
                        let (para_text, expect_lit) = split_paragraph_double_colon(&item_para);
                        if !para_text.is_empty() {
                            out.push_str(&render_rst_inline(&para_text, targets));
                        }
                        item_para.clear();
                        *idx = next_nb;
                        if expect_lit {
                            if let Some(code) = consume_indented_literal_block(lines, idx) {
                                out.push_str(&format!(
                                    "<pre><code>{}</code></pre>",
                                    html_escape(&code)
                                ));
                            }
                        } else {
                            out.push('\n');
                            parse_rst_list(lines, idx, ci, targets, out);
                        }
                        continue;
                    }
                    break;
                }
                if leading_spaces(lines[next_nb]) > base_indent {
                    let (para_text, expect_lit) = split_paragraph_double_colon(&item_para);
                    if !para_text.is_empty() {
                        out.push_str(&render_rst_inline(&para_text, targets));
                    }
                    item_para.clear();
                    if expect_lit {
                        if let Some(code) = consume_indented_literal_block(lines, idx) {
                            out.push_str(&format!(
                                "<pre><code>{}</code></pre>",
                                html_escape(&code)
                            ));
                            continue;
                        }
                    }
                }
                break;
            }

            if let Some((ci, _, _)) = detect_rst_list_marker(line) {
                if ci <= base_indent {
                    break;
                }
                let (para_text, _) = split_paragraph_double_colon(&item_para);
                if !para_text.is_empty() {
                    out.push_str(&render_rst_inline(&para_text, targets));
                }
                item_para.clear();
                parse_rst_list(lines, idx, ci, targets, out);
                continue;
            }

            if leading_spaces(line) <= base_indent {
                break;
            }
            item_para.push(line.trim());
            *idx += 1;
        }

        if !item_para.is_empty() {
            let (para_text, expect_lit) = split_paragraph_double_colon(&item_para);
            if !para_text.is_empty() {
                out.push_str(&render_rst_inline(&para_text, targets));
            }
            if expect_lit {
                if let Some(code) = consume_indented_literal_block(lines, idx) {
                    out.push_str(&format!("<pre><code>{}</code></pre>", html_escape(&code)));
                }
            }
        }
        out.push_str("</li>\n");
    }

    out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
}

fn try_parse_rst_grid_table(
    lines: &[&str],
    idx: &mut usize,
    targets: &HashMap<String, String>,
    out: &mut String,
) -> bool {
    let cur = lines[*idx].trim();
    if !(cur.starts_with('+')
        && cur.ends_with('+')
        && cur.len() >= 5
        && cur.chars().all(|c| matches!(c, '+' | '-' | '=')))
    {
        return false;
    }
    let Some(next) = lines.get(*idx + 1).map(|s| s.trim()) else {
        return false;
    };
    if !(next.starts_with('|') && next.ends_with('|')) {
        return false;
    }

    let mut pos = *idx;
    let mut rows: Vec<(Vec<String>, bool)> = Vec::new();
    let mut cur_cells: Vec<String> = Vec::new();
    let mut saw_header_sep = false;

    while pos < lines.len() {
        let t = lines[pos].trim();
        if t.starts_with('+')
            && t.ends_with('+')
            && t.chars().all(|c| matches!(c, '+' | '-' | '='))
        {
            let is_header_sep = t.contains('=');
            if !cur_cells.is_empty() {
                rows.push((std::mem::take(&mut cur_cells), is_header_sep));
                if is_header_sep {
                    saw_header_sep = true;
                }
            }
            pos += 1;
            continue;
        }
        if t.starts_with('|') && t.ends_with('|') {
            let body = &t[1..t.len() - 1];
            let mut cols: Vec<String> = Vec::new();
            let mut cell = String::new();
            let mut chars = body.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' && matches!(chars.peek(), Some(&'|')) {
                    cell.push('|');
                    chars.next();
                    continue;
                }
                if c == '|' {
                    cols.push(cell.trim().to_string());
                    cell.clear();
                } else {
                    cell.push(c);
                }
            }
            cols.push(cell.trim().to_string());
            if cur_cells.is_empty() {
                cur_cells = cols;
            } else {
                for (dst, src) in cur_cells.iter_mut().zip(cols) {
                    if !src.is_empty() {
                        if !dst.is_empty() {
                            dst.push(' ');
                        }
                        dst.push_str(&src);
                    }
                }
            }
            pos += 1;
            continue;
        }
        break;
    }

    if rows.is_empty() {
        return false;
    }
    *idx = pos;
    out.push_str("<table>\n");
    for (r_idx, (cells, _)) in rows.iter().enumerate() {
        let cell_tag = if saw_header_sep && r_idx == 0 {
            "th"
        } else {
            "td"
        };
        out.push_str("<tr>");
        for cell in cells {
            out.push('<');
            out.push_str(cell_tag);
            out.push('>');
            out.push_str(&render_rst_inline(cell, targets));
            out.push_str("</");
            out.push_str(cell_tag);
            out.push('>');
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    true
}

fn render_rst_blocks(
    lines: &[&str],
    targets: &HashMap<String, String>,
    heading_styles: &mut Vec<(char, bool)>,
    out: &mut String,
) {
    let mut i = 0;
    while i < lines.len() {
        let raw_line = lines[i];
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if let Some(after_dots) = trimmed.strip_prefix("..") {
            let after_ws = after_dots.trim_start();
            if after_ws.starts_with('_') && after_ws.contains(':') {
                i += 1;
                while i < lines.len() && leading_spaces(lines[i]) > 0 {
                    i += 1;
                }
                continue;
            }
            if let Some(directive_rest) = after_ws
                .strip_prefix("code-block::")
                .or_else(|| after_ws.strip_prefix("sourcecode::"))
                .or_else(|| after_ws.strip_prefix("code::"))
                .or_else(|| after_ws.strip_prefix("parsed-literal::"))
            {
                let lang = directive_rest.trim();
                i += 1;
                while i < lines.len() {
                    let t = lines[i].trim();
                    if t.starts_with(':') && leading_spaces(lines[i]) > 0 {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if let Some(code) = consume_indented_literal_block(lines, &mut i) {
                    if lang.is_empty() {
                        out.push_str(&format!("<pre><code>{}</code></pre>\n", html_escape(&code)));
                    } else {
                        out.push_str(&format!(
                            "<pre><code class=\"language-{}\">{}</code></pre>\n",
                            html_escape(lang),
                            html_escape(&code)
                        ));
                    }
                }
                continue;
            }
            let mut matched_admon: Option<(&str, &str)> = None;
            for (kw, label) in [
                ("note::", "Note"),
                ("warning::", "Warning"),
                ("tip::", "Tip"),
                ("important::", "Important"),
                ("caution::", "Caution"),
            ] {
                if let Some(rest) = after_ws.strip_prefix(kw) {
                    matched_admon = Some((label, rest.trim()));
                    break;
                }
            }
            if let Some((label, first_part)) = matched_admon {
                let mut parts = Vec::new();
                if !first_part.is_empty() {
                    parts.push(first_part.to_string());
                }
                i += 1;
                while i < lines.len() {
                    if lines[i].trim().is_empty() {
                        let mut nb = i + 1;
                        while nb < lines.len() && lines[nb].trim().is_empty() {
                            nb += 1;
                        }
                        if nb < lines.len() && leading_spaces(lines[nb]) > 0 {
                            i = nb;
                            continue;
                        }
                        break;
                    }
                    if leading_spaces(lines[i]) > 0 {
                        parts.push(lines[i].trim().to_string());
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push_str(&format!(
                    "<blockquote><p><strong>{label}:</strong> {}</p></blockquote>\n",
                    render_rst_inline(&parts.join(" "), targets)
                ));
                continue;
            }
            if after_ws.is_empty() || after_dots.starts_with(' ') {
                i += 1;
                while i < lines.len()
                    && (lines[i].trim().is_empty() || leading_spaces(lines[i]) > 0)
                {
                    if lines[i].trim().is_empty() {
                        let mut nb = i + 1;
                        while nb < lines.len() && lines[nb].trim().is_empty() {
                            nb += 1;
                        }
                        if nb < lines.len() && leading_spaces(lines[nb]) > 0 {
                            i = nb;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }

        if let Some(ch) = rst_adornment_char(trimmed) {
            if i + 2 < lines.len() {
                let mid = lines[i + 1].trim();
                let bot = lines[i + 2].trim();
                if !mid.is_empty()
                    && rst_adornment_char(bot) == Some(ch)
                    && bot.chars().count() >= mid.chars().count().min(3)
                {
                    let style = (ch, true);
                    let lvl = heading_styles
                        .iter()
                        .position(|&s| s == style)
                        .unwrap_or_else(|| {
                            heading_styles.push(style);
                            heading_styles.len() - 1
                        })
                        .saturating_add(1)
                        .clamp(1, 6);
                    out.push_str(&format!(
                        "<h{lvl}>{}</h{lvl}>\n",
                        render_rst_inline(mid, targets)
                    ));
                    i += 3;
                    continue;
                }
            }
            if trimmed.chars().count() >= 4 {
                out.push_str("<hr/>\n");
                i += 1;
                continue;
            }
        }

        if leading_spaces(raw_line) == 0 && i + 1 < lines.len() {
            let next_trimmed = lines[i + 1].trim();
            if let Some(ch) = rst_adornment_char(next_trimmed) {
                let min_len = trimmed.chars().count().clamp(2, 3);
                if next_trimmed.chars().count() >= min_len
                    && (next_trimmed.chars().count() >= trimmed.chars().count()
                        || detect_rst_list_marker(raw_line).is_none())
                {
                    let style = (ch, false);
                    let lvl = heading_styles
                        .iter()
                        .position(|&s| s == style)
                        .unwrap_or_else(|| {
                            heading_styles.push(style);
                            heading_styles.len() - 1
                        })
                        .saturating_add(1)
                        .clamp(1, 6);
                    out.push_str(&format!(
                        "<h{lvl}>{}</h{lvl}>\n",
                        render_rst_inline(trimmed, targets)
                    ));
                    i += 2;
                    continue;
                }
            }
        }

        if try_parse_rst_grid_table(lines, &mut i, targets, out) {
            continue;
        }

        if let Some((indent, _, _)) = detect_rst_list_marker(raw_line) {
            parse_rst_list(lines, &mut i, indent, targets, out);
            continue;
        }

        if leading_spaces(raw_line) > 0 {
            if let Some(quote_block) = consume_indented_literal_block(lines, &mut i) {
                let quote_lines: Vec<&str> = quote_block.lines().collect();
                out.push_str("<blockquote>\n");
                render_rst_blocks(&quote_lines, targets, heading_styles, out);
                out.push_str("</blockquote>\n");
                continue;
            }
        }

        let mut para_lines: Vec<&str> = Vec::new();
        while i < lines.len() {
            let line = lines[i];
            let t = line.trim();
            if t.is_empty() {
                break;
            }
            if !para_lines.is_empty()
                && (detect_rst_list_marker(line).is_some()
                    || rst_adornment_char(t)
                        .map(|_| t.chars().count() >= 4)
                        .unwrap_or(false))
            {
                break;
            }
            if !para_lines.is_empty() && i + 1 < lines.len() {
                let nt = lines[i + 1].trim();
                if rst_adornment_char(nt).is_some() && nt.chars().count() >= t.chars().count().min(3)
                {
                    break;
                }
            }
            para_lines.push(line);
            i += 1;
        }
        let (para_text, expect_lit) = split_paragraph_double_colon(&para_lines);
        if !para_text.is_empty() {
            out.push_str(&format!(
                "<p>{}</p>\n",
                render_rst_inline(&para_text, targets)
            ));
        }
        if expect_lit {
            if let Some(code) = consume_indented_literal_block(lines, &mut i) {
                out.push_str(&format!("<pre><code>{}</code></pre>\n", html_escape(&code)));
            }
        }
    }
}

pub fn rst_to_html(title: &str, rst_input: &str) -> String {
    let normalized = rst_input.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();

    let mut targets: HashMap<String, String> = HashMap::new();
    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if let Some(after_dots) = trimmed.strip_prefix("..") {
            let after_ws = after_dots.trim_start();
            if let Some(rest) = after_ws.strip_prefix('_') {
                if let Some(colon_pos) = rest.find(':') {
                    let key = normalize_rst_ref_key(&rest[..colon_pos]);
                    let mut url = rest[colon_pos + 1..].trim().to_string();
                    if url.is_empty()
                        && idx + 1 < lines.len()
                        && leading_spaces(lines[idx + 1]) > 0
                    {
                        url = lines[idx + 1].trim().to_string();
                    }
                    if !key.is_empty() && !url.is_empty() {
                        targets.insert(key, url);
                    }
                }
            }
        }
    }

    let mut body = String::new();
    let mut heading_styles: Vec<(char, bool)> = Vec::new();
    render_rst_blocks(&lines, &targets, &mut heading_styles, &mut body);

    let safe_title = if title.trim().is_empty() {
        "reStructuredText Document"
    } else {
        title.trim()
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:11.5pt;line-height:1.5;color:#111;}}h1,h2,h3{{margin:14pt 0 6pt 0;}}pre,code{{font-family:Menlo,monospace;font-size:10pt;background:#f4f4f6;}}blockquote{{border-left:3px solid #bbb;padding-left:10pt;color:#444;}}table{{border-collapse:collapse;width:100%;}}th,td{{border:1px solid #ccc;padding:4pt 6pt;}}</style>\
        </head><body>\n{body}</body></html>",
        html_escape(safe_title)
    )
}

pub fn read_rst_to_html(path: &Path) -> Result<(String, String), EngineError> {
    let raw = read_text_bounded_with_utf16(path, MAX_TEXT_READ_BYTES, "RST")?;
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("reStructuredText Document")
        .to_string();
    let html = rst_to_html(&title, &raw);
    Ok((title, html))
}

fn wrap_rst_inline(raw: &str, open: &str, close: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lead = raw.chars().take_while(|c| c.is_whitespace()).count();
    let trail = raw.chars().rev().take_while(|c| c.is_whitespace()).count();
    let lead_ws: String = raw.chars().take(lead).collect();
    let trail_ws: String = raw
        .chars()
        .rev()
        .take(trail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{lead_ws}{open}{trimmed}{close}{trail_ws}")
}

pub fn html_to_rst(html_input: &str) -> String {
    let body_slice = extract_html_body_slice(html_input);
    let mut out = String::new();
    let mut skip_tag: Option<(String, usize)> = None;

    let mut heading_level: Option<usize> = None;
    let mut heading_buf = String::new();

    let mut in_pre = false;
    let mut pre_buf = String::new();
    let mut in_blockquote = false;
    let mut quote_buf = String::new();

    enum RstInlineFrame {
        Style {
            tag: &'static str,
            open: &'static str,
            close: &'static str,
            buf: String,
        },
        Link {
            href: Option<String>,
            buf: String,
        },
    }

    impl RstInlineFrame {
        fn buf_mut(&mut self) -> &mut String {
            match self {
                Self::Style { buf, .. } | Self::Link { buf, .. } => buf,
            }
        }

        fn matches_close(&self, tag: &str) -> bool {
            match self {
                Self::Style { tag: t, .. } => match *t {
                    "strong" => matches!(tag, "strong" | "b"),
                    "em" => matches!(tag, "em" | "i"),
                    "code" => matches!(tag, "code" | "kbd" | "samp" | "tt"),
                    _ => false,
                },
                Self::Link { .. } => tag == "a",
            }
        }

        fn render(self) -> String {
            match self {
                Self::Style { open, close, buf, .. } => wrap_rst_inline(&buf, open, close),
                Self::Link { href, buf } => {
                    let label = buf.trim().to_string();
                    if let Some(url) = href {
                        let display = if label.is_empty() { url.as_str() } else { label.as_str() };
                        format!("`{display} <{url}>`_")
                    } else {
                        label
                    }
                }
            }
        }
    }

    let mut frame_stack: Vec<RstInlineFrame> = Vec::new();
    let mut list_stack: Vec<(bool, usize)> = Vec::new();

    let mut in_table = false;
    let mut table_rows: Vec<(Vec<String>, bool)> = Vec::new();
    let mut cur_row: Vec<String> = Vec::new();
    let mut cur_row_has_th = false;
    let mut in_cell = false;
    let mut cur_cell = String::new();

    #[allow(clippy::too_many_arguments)]
    fn push_target(
        text: &str,
        heading_level: &Option<usize>,
        heading_buf: &mut String,
        in_cell: bool,
        cur_cell: &mut String,
        frame_stack: &mut [RstInlineFrame],
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        if let Some(frame) = frame_stack.last_mut() {
            frame.buf_mut().push_str(text);
        } else if heading_level.is_some() {
            heading_buf.push_str(text);
        } else if in_cell {
            cur_cell.push_str(text);
        } else if in_blockquote {
            quote_buf.push_str(text);
        } else {
            out.push_str(text);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_rendered_inline(
        rendered: &str,
        heading_level: &Option<usize>,
        heading_buf: &mut String,
        in_cell: bool,
        cur_cell: &mut String,
        frame_stack: &mut [RstInlineFrame],
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        if let Some(parent) = frame_stack.last_mut() {
            parent.buf_mut().push_str(rendered);
        } else if heading_level.is_some() {
            heading_buf.push_str(rendered);
        } else if in_cell {
            cur_cell.push_str(rendered);
        } else if in_blockquote {
            quote_buf.push_str(rendered);
        } else {
            out.push_str(rendered);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flush_all_frames(
        frame_stack: &mut Vec<RstInlineFrame>,
        heading_level: &Option<usize>,
        heading_buf: &mut String,
        in_cell: bool,
        cur_cell: &mut String,
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        while let Some(frame) = frame_stack.pop() {
            let rendered = frame.render();
            emit_rendered_inline(
                &rendered,
                heading_level,
                heading_buf,
                in_cell,
                cur_cell,
                frame_stack,
                in_blockquote,
                quote_buf,
                out,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn close_matching_frame(
        tag: &str,
        frame_stack: &mut Vec<RstInlineFrame>,
        heading_level: &Option<usize>,
        heading_buf: &mut String,
        in_cell: bool,
        cur_cell: &mut String,
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        if let Some(pos) = frame_stack.iter().rposition(|f| f.matches_close(tag)) {
            while frame_stack.len() > pos {
                if let Some(frame) = frame_stack.pop() {
                    let rendered = frame.render();
                    emit_rendered_inline(
                        &rendered,
                        heading_level,
                        heading_buf,
                        in_cell,
                        cur_cell,
                        frame_stack,
                        in_blockquote,
                        quote_buf,
                        out,
                    );
                }
            }
        }
    }

    fn emit_pre_block(in_blockquote: bool, quote_buf: &mut String, out: &mut String, pre: &str) {
        let norm = pre.replace("\r\n", "\n").replace('\r', "\n");
        let trimmed_end = norm.strip_suffix('\n').unwrap_or(&norm);
        let dst = if in_blockquote { quote_buf } else { out };
        if !dst.ends_with("\n\n") && !dst.is_empty() {
            if !dst.ends_with('\n') {
                dst.push('\n');
            }
            dst.push('\n');
        }
        dst.push_str("::\n\n");
        for line in trimmed_end.split('\n') {
            if line.trim().is_empty() {
                dst.push('\n');
            } else {
                dst.push_str("    ");
                dst.push_str(line);
                dst.push('\n');
            }
        }
        dst.push('\n');
    }

    fn emit_heading(
        lvl: usize,
        heading_buf: &str,
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        let clean = heading_buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() {
            return;
        }
        let underline_ch = match lvl {
            1 => '=',
            2 => '-',
            3 => '~',
            4 => '^',
            5 => '"',
            _ => '+',
        };
        let underline_len = clean.chars().count().max(3);
        let dst = if in_blockquote { quote_buf } else { out };
        if !dst.is_empty() && !dst.ends_with("\n\n") {
            if !dst.ends_with('\n') {
                dst.push('\n');
            }
            dst.push('\n');
        }
        dst.push_str(&clean);
        dst.push('\n');
        dst.push_str(&underline_ch.to_string().repeat(underline_len));
        dst.push_str("\n\n");
    }

    fn emit_blockquote(quote_buf: &str, out: &mut String) {
        let qtext = quote_buf.trim_matches(|c| c == '\n' || c == '\r');
        if qtext.trim().is_empty() {
            return;
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        let mut in_q_literal = false;
        for qline in qtext.lines() {
            let trimmed_end = qline.trim_end();
            if trimmed_end == "::" {
                in_q_literal = true;
                out.push_str("    ::\n");
            } else if trimmed_end.trim().is_empty() {
                out.push('\n');
            } else if in_q_literal && qline.starts_with("  ") {
                out.push_str("    ");
                out.push_str(trimmed_end);
                out.push('\n');
            } else {
                in_q_literal = false;
                out.push_str("    ");
                out.push_str(qline.trim());
                out.push('\n');
            }
        }
        out.push('\n');
    }

    fn emit_table(
        table_rows: &[(Vec<String>, bool)],
        in_blockquote: bool,
        quote_buf: &mut String,
        out: &mut String,
    ) {
        let ncols = table_rows.iter().map(|(r, _)| r.len()).max().unwrap_or(0);
        if ncols == 0 {
            return;
        }
        let mut widths = vec![3usize; ncols];
        for (cells, _) in table_rows {
            for (c_idx, cell) in cells.iter().enumerate() {
                widths[c_idx] = widths[c_idx].max(cell.chars().count());
            }
        }
        let make_border = |ch: char| -> String {
            let mut s = String::from("+");
            for w in &widths {
                s.push(ch);
                s.push_str(&ch.to_string().repeat(*w));
                s.push(ch);
                s.push('+');
            }
            s
        };
        let thin = make_border('-');
        let thick = make_border('=');
        let dst = if in_blockquote { quote_buf } else { out };
        if !dst.is_empty() && !dst.ends_with("\n\n") {
            if !dst.ends_with('\n') {
                dst.push('\n');
            }
            dst.push('\n');
        }
        dst.push_str(&thin);
        dst.push('\n');
        for (r_idx, (cells, is_th)) in table_rows.iter().enumerate() {
            dst.push('|');
            for (c_idx, width) in widths.iter().enumerate().take(ncols) {
                let txt = cells.get(c_idx).map(|s| s.as_str()).unwrap_or("");
                let pad = width.saturating_sub(txt.chars().count());
                dst.push(' ');
                dst.push_str(txt);
                dst.push_str(&" ".repeat(pad));
                dst.push_str(" |");
            }
            dst.push('\n');
            let use_thick = *is_th && r_idx == 0;
            dst.push_str(if use_thick { &thick } else { &thin });
            dst.push('\n');
        }
        dst.push('\n');
    }

    let bytes = body_slice.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if body_slice[i..].starts_with("<!--") {
                if let Some(end_rel) = body_slice[i + 4..].find("-->") {
                    i = i + 4 + end_rel + 3;
                } else {
                    break;
                }
                continue;
            }
            if body_slice[i..].starts_with("<![CDATA[") {
                let (cdata_end, next_i) = if let Some(end_rel) = body_slice[i + 9..].find("]]>") {
                    (i + 9 + end_rel, i + 9 + end_rel + 3)
                } else {
                    (bytes.len(), bytes.len())
                };
                if skip_tag.is_none() {
                    let s = &body_slice[i + 9..cdata_end];
                    if in_pre {
                        pre_buf.push_str(s);
                    } else {
                        push_target(
                            s,
                            &heading_level,
                            &mut heading_buf,
                            in_cell,
                            &mut cur_cell,
                            &mut frame_stack,
                            in_blockquote,
                            &mut quote_buf,
                            &mut out,
                        );
                    }
                }
                i = next_i;
                continue;
            }
            if !matches!(
                bytes.get(i + 1),
                Some(b) if b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')
            ) {
                if skip_tag.is_none() {
                    if in_pre {
                        pre_buf.push('<');
                    } else {
                        push_target(
                            "<",
                            &heading_level,
                            &mut heading_buf,
                            in_cell,
                            &mut cur_cell,
                            &mut frame_stack,
                            in_blockquote,
                            &mut quote_buf,
                            &mut out,
                        );
                    }
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
                break;
            }

            let raw_tag = body_slice[i + 1..j].trim();
            i = j + 1;
            if raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }
            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let self_closing = !is_close && has_xml_self_closing_slash(inner);
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if !is_close
                && matches!(tag_name.as_str(), "script" | "style" | "noscript" | "template")
                && !self_closing
            {
                i = find_raw_close_tag(body_slice, i, &tag_name).unwrap_or(bytes.len());
                continue;
            }
            if let Some((ref skipping, ref mut depth)) = skip_tag {
                if &tag_name == skipping {
                    if is_close {
                        *depth -= 1;
                        if *depth == 0 {
                            skip_tag = None;
                        }
                    } else if !self_closing {
                        *depth += 1;
                    }
                }
                continue;
            }
            if !is_close
                && matches!(tag_name.as_str(), "head" | "svg" | "iframe" | "object" | "embed")
                && !self_closing
            {
                skip_tag = Some((tag_name, 1));
                continue;
            }

            if in_pre {
                if is_close && tag_name == "pre" {
                    emit_pre_block(in_blockquote, &mut quote_buf, &mut out, &pre_buf);
                    pre_buf.clear();
                    in_pre = false;
                } else if !is_close && tag_name == "br" {
                    pre_buf.push('\n');
                }
                continue;
            }

            match (is_close, tag_name.as_str()) {
                (false, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    let lvl = tag_name[1..].parse::<usize>().unwrap_or(1);
                    heading_level = Some(lvl);
                    heading_buf.clear();
                }
                (true, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    if let Some(lvl) = heading_level.take() {
                        emit_heading(lvl, &heading_buf, in_blockquote, &mut quote_buf, &mut out);
                    }
                    heading_buf.clear();
                }
                (false, "pre") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    in_pre = true;
                    pre_buf.clear();
                }
                (false, "blockquote") => {
                    if !in_blockquote {
                        in_blockquote = true;
                        quote_buf.clear();
                    }
                }
                (true, "blockquote") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    if in_blockquote {
                        in_blockquote = false;
                        emit_blockquote(&quote_buf, &mut out);
                        quote_buf.clear();
                    }
                }
                (false, "ul") => {
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    if !dst.is_empty() && !dst.ends_with("\n\n") {
                        if !dst.ends_with('\n') {
                            dst.push('\n');
                        }
                        dst.push('\n');
                    }
                    list_stack.push((false, 0));
                }
                (false, "ol") => {
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    if !dst.is_empty() && !dst.ends_with("\n\n") {
                        if !dst.ends_with('\n') {
                            dst.push('\n');
                        }
                        dst.push('\n');
                    }
                    list_stack.push((true, 0));
                }
                (true, "ul" | "ol") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    list_stack.pop();
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    dst.push_str("\n\n");
                }
                (false, "li") => {
                    let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    if !dst.is_empty() && !dst.ends_with('\n') {
                        dst.push('\n');
                    }
                    if let Some((true, count)) = list_stack.last_mut() {
                        *count += 1;
                        dst.push_str(&format!("{indent}{count}. "));
                    } else {
                        dst.push_str(&format!("{indent}- "));
                    }
                }
                (true, "li") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    if !dst.ends_with('\n') {
                        dst.push('\n');
                    }
                }
                (false, "strong" | "b") => {
                    frame_stack.push(RstInlineFrame::Style {
                        tag: "strong",
                        open: "**",
                        close: "**",
                        buf: String::new(),
                    });
                }
                (true, "strong" | "b") => close_matching_frame(
                    "strong",
                    &mut frame_stack,
                    &heading_level,
                    &mut heading_buf,
                    in_cell,
                    &mut cur_cell,
                    in_blockquote,
                    &mut quote_buf,
                    &mut out,
                ),
                (false, "em" | "i") => {
                    frame_stack.push(RstInlineFrame::Style {
                        tag: "em",
                        open: "*",
                        close: "*",
                        buf: String::new(),
                    });
                }
                (true, "em" | "i") => close_matching_frame(
                    "em",
                    &mut frame_stack,
                    &heading_level,
                    &mut heading_buf,
                    in_cell,
                    &mut cur_cell,
                    in_blockquote,
                    &mut quote_buf,
                    &mut out,
                ),
                (false, "code" | "kbd" | "samp" | "tt") => {
                    frame_stack.push(RstInlineFrame::Style {
                        tag: "code",
                        open: "``",
                        close: "``",
                        buf: String::new(),
                    });
                }
                (true, "code" | "kbd" | "samp" | "tt") => close_matching_frame(
                    "code",
                    &mut frame_stack,
                    &heading_level,
                    &mut heading_buf,
                    in_cell,
                    &mut cur_cell,
                    in_blockquote,
                    &mut quote_buf,
                    &mut out,
                ),
                (false, "a") => {
                    let href = extract_attr(inner, "href").and_then(|raw| {
                        let cleaned = decode_html_entities(&raw).trim().to_string();
                        if !cleaned.is_empty() && is_safe_xhtml_url(&cleaned, false) {
                            Some(cleaned)
                        } else {
                            None
                        }
                    });
                    frame_stack.push(RstInlineFrame::Link {
                        href,
                        buf: String::new(),
                    });
                }
                (true, "a") => close_matching_frame(
                    "a",
                    &mut frame_stack,
                    &heading_level,
                    &mut heading_buf,
                    in_cell,
                    &mut cur_cell,
                    in_blockquote,
                    &mut quote_buf,
                    &mut out,
                ),
                (false, "br") => {
                    if heading_level.is_some() {
                        heading_buf.push(' ');
                    } else if in_cell {
                        cur_cell.push(' ');
                    } else if !list_stack.is_empty() || !frame_stack.is_empty() {
                        push_target(
                            " ",
                            &heading_level,
                            &mut heading_buf,
                            in_cell,
                            &mut cur_cell,
                            &mut frame_stack,
                            in_blockquote,
                            &mut quote_buf,
                            &mut out,
                        );
                    } else {
                        let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                        dst.push_str("\n\n");
                    }
                }
                (false, "hr") => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    dst.push_str("\n----\n\n");
                }
                (true, "p" | "div") if !in_cell => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    let dst = if in_blockquote { &mut quote_buf } else { &mut out };
                    if !dst.is_empty() && !dst.ends_with("\n\n") {
                        if !dst.ends_with('\n') {
                            dst.push('\n');
                        }
                        dst.push('\n');
                    }
                }
                (false, "table") => {
                    in_table = true;
                    table_rows.clear();
                }
                (true, "table") if in_table => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    in_table = false;
                    emit_table(&table_rows, in_blockquote, &mut quote_buf, &mut out);
                    table_rows.clear();
                }
                (false, "tr") if in_table => {
                    cur_row.clear();
                    cur_row_has_th = false;
                }
                (true, "tr") if in_table => {
                    if !cur_row.is_empty() {
                        table_rows.push((std::mem::take(&mut cur_row), cur_row_has_th));
                    }
                }
                (false, "th") if in_table => {
                    in_cell = true;
                    cur_cell.clear();
                    cur_row_has_th = true;
                }
                (false, "td") if in_table => {
                    in_cell = true;
                    cur_cell.clear();
                }
                (true, "th" | "td") if in_table => {
                    flush_all_frames(
                        &mut frame_stack,
                        &heading_level,
                        &mut heading_buf,
                        in_cell,
                        &mut cur_cell,
                        in_blockquote,
                        &mut quote_buf,
                        &mut out,
                    );
                    in_cell = false;
                    let cleaned = cur_cell
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .replace('|', "\\|");
                    cur_row.push(cleaned);
                    cur_cell.clear();
                }
                _ => {}
            }
            continue;
        }

        let next_tag_rel = body_slice[i..].find('<').unwrap_or(body_slice.len() - i);
        let raw_chunk = &body_slice[i..i + next_tag_rel];
        i += next_tag_rel;
        if skip_tag.is_some() {
            continue;
        }
        let decoded_chunk = decode_html_entities_inner(raw_chunk, true);
        if in_pre {
            pre_buf.push_str(&decoded_chunk);
        } else {
            let normalized: String = decoded_chunk
                .chars()
                .filter_map(|c| match c {
                    '\r' => None,
                    '\n' => Some(' '),
                    other => Some(other),
                })
                .collect();
            if !normalized.is_empty() {
                push_target(
                    &normalized,
                    &heading_level,
                    &mut heading_buf,
                    in_cell,
                    &mut cur_cell,
                    &mut frame_stack,
                    in_blockquote,
                    &mut quote_buf,
                    &mut out,
                );
            }
        }
    }

    if in_pre {
        emit_pre_block(in_blockquote, &mut quote_buf, &mut out, &pre_buf);
    }
    flush_all_frames(
        &mut frame_stack,
        &heading_level,
        &mut heading_buf,
        in_cell,
        &mut cur_cell,
        in_blockquote,
        &mut quote_buf,
        &mut out,
    );
    if let Some(lvl) = heading_level.take() {
        emit_heading(lvl, &heading_buf, in_blockquote, &mut quote_buf, &mut out);
    }
    if in_cell && in_table {
        let cleaned = cur_cell
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('|', "\\|");
        cur_row.push(cleaned);
    }
    if in_table {
        if !cur_row.is_empty() {
            table_rows.push((std::mem::take(&mut cur_row), cur_row_has_th));
        }
        emit_table(&table_rows, in_blockquote, &mut quote_buf, &mut out);
    }
    if in_blockquote {
        emit_blockquote(&quote_buf, &mut out);
    }

    let cleaned_out = out.replace('\u{00A0}', " ");
    let mut collapsed = String::new();
    let mut blank_run = 0usize;
    let mut in_indented_block = false;

    for raw_line in cleaned_out.lines() {
        let mut line = raw_line.trim_end().to_string();
        if line == "::" || line == "    ::" {
            in_indented_block = true;
            blank_run = 0;
            collapsed.push_str(&line);
            collapsed.push('\n');
            continue;
        }
        let is_indented = line.starts_with("  ");
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !collapsed.is_empty() {
                collapsed.push('\n');
            }
            continue;
        }
        if !in_indented_block && !is_indented {
            line = line.trim_start().to_string();
        } else if in_indented_block && !is_indented {
            in_indented_block = false;
        }
        blank_run = 0;
        collapsed.push_str(&line);
        collapsed.push('\n');
    }

    let result = collapsed.trim().to_string();
    if result.is_empty() {
        String::new()
    } else {
        format!("{result}\n")
    }
}

pub fn write_native_fb2_or_rst(
    title: &str,
    html_input: &str,
    target_fmt: &str,
    output: &Path,
) -> Result<(), EngineError> {
    match target_fmt.to_ascii_uppercase().as_str() {
        "FB2" => {
            fs::write(output, write_fb2_from_html(title, html_input))?;
            Ok(())
        }
        "RST" => {
            fs::write(output, html_to_rst(html_input))?;
            Ok(())
        }
        other => Err(EngineError::Unsupported(format!(
            "Unsupported native target format: {other}"
        ))),
    }
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
        if matches!(fmt.as_str(), "FB2" | "RST") {
            if tools.pandoc.is_some() {
                match convert_via_pandoc(source, "markdown", &fmt, &output, staging_dir, tools, cancel)
                {
                    Ok(()) => return Ok(vec![output]),
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            }
            write_native_fb2_or_rst(source_title, &html_str, &fmt, &output)?;
            return Ok(vec![output]);
        }
        prepared_input = html_path;
        prepared_ext = "html".to_string();
    } else {
        prepared_input = source.to_path_buf();
        prepared_ext = ext.clone();
    }

    // 4) Native pure-Rust EPUB 3 reader (EPUB -> TXT/MD/HTML/RTF/DOCX/ODT/PDF/FB2/RST)
    if prepared_ext == "epub" {
        let (book_title, combined_html) = read_epub_to_html(&prepared_input)?;
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
        fs::write(&html_mid, &combined_html)?;
        if matches!(fmt.as_str(), "FB2" | "RST") {
            if tools.pandoc.is_some() {
                match convert_via_pandoc(&html_mid, "html", &fmt, &output, staging_dir, tools, cancel) {
                    Ok(()) => return Ok(vec![output]),
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            }
            write_native_fb2_or_rst(&book_title, &combined_html, &fmt, &output)?;
            return Ok(vec![output]);
        }
        bridge::convert_doc(&html_mid, &fmt, &output, &prefs.pdf_page_size, Some(cancel))?;
        return Ok(vec![output]);
    }

    // 5) Native pure-Rust EPUB 3 writer (TXT, HTML, RTF, DOC, DOCX, ODT, FB2, RST -> EPUB)
    if fmt == "EPUB" {
        let mut epub_title = source_title.to_string();
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
            let mut pandoc_html: Option<String> = None;
            if tools.pandoc.is_some() {
                let html_mid = staging_dir.join("pandoc-to-epub.html");
                match convert_via_pandoc(
                    &prepared_input,
                    &prepared_ext,
                    "HTML",
                    &html_mid,
                    staging_dir,
                    tools,
                    cancel,
                ) {
                    Ok(()) => {
                        pandoc_html = Some(read_html_file_safe(&html_mid, MAX_HTML_READ_BYTES)?);
                    }
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            }
            if let Some(html) = pandoc_html {
                html
            } else if prepared_ext == "fb2" {
                let (t, html) = read_fb2_to_html(&prepared_input)?;
                epub_title = t;
                html
            } else {
                let (t, html) = read_rst_to_html(&prepared_input)?;
                epub_title = t;
                html
            }
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
        write_epub_from_html(&epub_title, &html_for_epub, &output)?;
        return Ok(vec![output]);
    }

    // 6) FB2 and RST as source or target (Pandoc preferred when installed, zero-Pandoc native Rust fallback)
    if matches!(prepared_ext.as_str(), "fb2" | "rst") || matches!(fmt.as_str(), "FB2" | "RST") {
        if tools.pandoc.is_some() {
            let from_pandoc = match prepared_ext.as_str() {
                "md" | "markdown" => "markdown",
                "htm" | "html" => "html",
                "txt" => "plain",
                other => other,
            };
            if matches!(fmt.as_str(), "FB2" | "RST") && prepared_ext == "txt" {
                let txt_raw = read_text_bounded_with_utf16(&prepared_input, MAX_TEXT_READ_BYTES, "TXT")?;
                let html_path = staging_dir.join("txt-wrap.html");
                fs::write(&html_path, plain_text_to_html("FLIP Document", &txt_raw))?;
                match convert_via_pandoc(&html_path, "html", &fmt, &output, staging_dir, tools, cancel) {
                    Ok(()) => return Ok(vec![output]),
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            } else if matches!(prepared_ext.as_str(), "fb2" | "rst") && fmt == "PDF" {
                let html_mid = staging_dir.join("pandoc-mid.html");
                match convert_via_pandoc(&prepared_input, from_pandoc, "HTML", &html_mid, staging_dir, tools, cancel) {
                    Ok(()) => {
                        bridge::convert_doc(&html_mid, "PDF", &output, &prefs.pdf_page_size, Some(cancel))?;
                        return Ok(vec![output]);
                    }
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            } else {
                match convert_via_pandoc(&prepared_input, from_pandoc, &fmt, &output, staging_dir, tools, cancel) {
                    Ok(()) => return Ok(vec![output]),
                    Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                    Err(_) => {}
                }
            }
        }

        if matches!(prepared_ext.as_str(), "fb2" | "rst") {
            let (doc_title, html_content) = if prepared_ext == "fb2" {
                read_fb2_to_html(&prepared_input)?
            } else {
                read_rst_to_html(&prepared_input)?
            };
            match fmt.as_str() {
                "HTML" => {
                    fs::write(&output, html_content)?;
                    return Ok(vec![output]);
                }
                "MD" => {
                    fs::write(&output, html_to_markdown(&html_content))?;
                    return Ok(vec![output]);
                }
                "TXT" => {
                    fs::write(&output, html_to_plain_text(&html_content))?;
                    return Ok(vec![output]);
                }
                "RST" => {
                    fs::write(&output, html_to_rst(&html_content))?;
                    return Ok(vec![output]);
                }
                "FB2" => {
                    fs::write(&output, write_fb2_from_html(&doc_title, &html_content))?;
                    return Ok(vec![output]);
                }
                _ => {
                    let html_mid = staging_dir.join("native-mid.html");
                    fs::write(&html_mid, html_content)?;
                    bridge::convert_doc(&html_mid, &fmt, &output, &prefs.pdf_page_size, Some(cancel))?;
                    return Ok(vec![output]);
                }
            }
        }

        let html_for_target = if prepared_ext == "txt" {
            let txt_raw = read_text_bounded_with_utf16(&prepared_input, MAX_TEXT_READ_BYTES, "TXT")?;
            plain_text_to_html(source_title, &txt_raw)
        } else if matches!(prepared_ext.as_str(), "html" | "htm") {
            read_html_file_safe(&prepared_input, MAX_HTML_READ_BYTES)?
        } else {
            let html_mid = staging_dir.join("doc-to-fb2-rst.html");
            bridge::convert_doc(
                &prepared_input,
                "HTML",
                &html_mid,
                &prefs.pdf_page_size,
                Some(cancel),
            )?;
            read_html_file_safe(&html_mid, MAX_HTML_READ_BYTES)?
        };
        write_native_fb2_or_rst(source_title, &html_for_target, &fmt, &output)?;
        return Ok(vec![output]);
    }

    // 7) Target is MD
    if fmt == "MD" {
        if tools.pandoc.is_some() && matches!(prepared_ext.as_str(), "html" | "htm" | "docx" | "odt" | "rtf") {
            let from_pd = if prepared_ext == "htm" { "html" } else { prepared_ext.as_str() };
            match convert_via_pandoc(&prepared_input, from_pd, "MD", &output, staging_dir, tools, cancel) {
                Ok(()) => {
                    let has_content = fs::read_to_string(&output)
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false);
                    if has_content || !matches!(prepared_ext.as_str(), "html" | "htm") {
                        return Ok(vec![output]);
                    }
                }
                Err(err) if err.is_cancelled() => return Err(EngineError::Cancelled),
                Err(_) => {}
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
        let html_raw = read_html_file_safe(&html_path, MAX_HTML_READ_BYTES)?;
        let md = html_to_markdown_with_fallback(&html_raw, source_title);
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

    if matches!(prepared_ext.as_str(), "html" | "htm") && fmt == "TXT" {
        let bridge_err = match bridge::convert_doc(
            &prepared_input,
            &fmt,
            &output,
            &prefs.pdf_page_size,
            Some(cancel),
        ) {
            Ok(_) => None,
            Err(bridge::BridgeError::Cancelled) => return Err(EngineError::Cancelled),
            Err(err) => Some(err),
        };
        let has_content = fs::read_to_string(&output)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if bridge_err.is_none() && has_content {
            return Ok(vec![output]);
        }
        let html_raw = read_html_file_safe(&prepared_input, MAX_HTML_READ_BYTES)?;
        if !html_raw.trim().is_empty() {
            let txt = html_to_plain_text_with_fallback(&html_raw, source_title);
            fs::write(&output, txt)?;
            return Ok(vec![output]);
        }
        if let Some(err) = bridge_err {
            return Err(err.into());
        }
        return Ok(vec![output]);
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

    #[test]
    fn fb2_reader_extracts_metadata_sections_poems_links_and_inline_markup() {
        let fb2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
  <description>
    <title-info>
      <genre>sci_fi</genre>
      <book-title>Zero Pandoc Novel</book-title>
    </title-info>
    <src-title-info>
      <book-title>Ignored Source Title</book-title>
    </src-title-info>
  </description>
  <body>
    <section>
      <title><p>Part One</p><p>Subtitle Line</p></title>
      <subtitle>Prelude</subtitle>
      <epigraph><p>First line quote.</p></epigraph>
      <p>Hello <strong>bold</strong>, <emphasis>italic</emphasis>, <code>let x = 1;</code>, and <a l:href="https://example.com/docs">docs</a> + <a l:href="javascript:alert(1)">unsafe</a>.</p>
      <empty-line/>
      <poem>
        <stanza>
          <v>Line one of verse</v>
          <v>Line two of verse</v>
        </stanza>
      </poem>
    </section>
  </body>
  <binary id="cover.png" content-type="image/png">BASE64_SHOULD_NOT_LEAK</binary>
</FictionBook>"#;

        let (title, html) = parse_fb2_to_html(fb2, "Fallback").unwrap();
        assert_eq!(title, "Zero Pandoc Novel");
        assert!(html.contains("<h1>Part One<br/>Subtitle Line</h1>"), "html={html}");
        assert!(html.contains("<h2>Prelude</h2>"), "html={html}");
        assert!(html.contains("<blockquote>"), "html={html}");
        assert!(html.contains("<strong>bold</strong>"), "html={html}");
        assert!(html.contains("<em>italic</em>"), "html={html}");
        assert!(html.contains("<code>let x = 1;</code>"), "html={html}");
        assert!(html.contains("<a href=\"https://example.com/docs\">docs</a>"), "html={html}");
        assert!(!html.contains("javascript:alert"), "unsafe link leaked: {html}");
        assert!(html.contains("Line one of verse<br/>Line two of verse"), "html={html}");
        assert!(!html.contains("BASE64_SHOULD_NOT_LEAK"), "binary leaked: {html}");

        let plain = html_to_plain_text(&html);
        assert!(plain.contains("Part One"), "plain={plain}");
        assert!(plain.contains("Line one of verse"), "plain={plain}");
        assert!(plain.contains("Line two of verse"), "plain={plain}");
        assert!(!plain.contains("Ignored Source Title"), "plain={plain}");
    }

    #[test]
    fn fb2_writer_produces_valid_xml_and_splits_code_block_lines() {
        let md = "# Native Book\n\nIntro paragraph with **strong**, *em*, `code`, and [Link](https://example.org).\n\n> Quoted line\n\n- Alpha\n- Beta\n\n```rust\nfn main() {\n\n    println!(\"hi & bye\");\n}\n```\n";
        let html = markdown_to_html("Native Book", md);
        let fb2_xml = write_fb2_from_html("Native Book", &html);

        assert!(fb2_xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(fb2_xml.contains("xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\""));
        assert!(fb2_xml.contains("<book-title>Native Book</book-title>"));
        assert!(fb2_xml.contains("<p><code>fn main() {</code></p>"), "fb2={fb2_xml}");
        assert!(fb2_xml.contains("<empty-line/>"), "fb2={fb2_xml}");
        assert!(fb2_xml.contains("<p><code>    println!(&quot;hi &amp; bye&quot;);</code></p>") || fb2_xml.contains("<p><code>    println!(\"hi &amp; bye\");</code></p>"), "fb2={fb2_xml}");
        assert!(fb2_xml.contains("<a l:href=\"https://example.org\">Link</a>"), "fb2={fb2_xml}");

        // Validate strict XML well-formedness via quick-xml
        let mut reader = Reader::from_str(&fb2_xml);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("Generated FB2 XML is invalid: {e}\n{fb2_xml}"),
            }
            buf.clear();
        }

        // Roundtrip readback retains all lines
        let (title, roundtrip_html) = parse_fb2_to_html(&fb2_xml, "Fallback").unwrap();
        assert_eq!(title, "Native Book");
        let plain = html_to_plain_text(&roundtrip_html);
        assert!(plain.contains("fn main() {"), "plain={plain}");
        assert!(plain.contains("println!(\"hi & bye\");"), "plain={plain}");
        assert!(plain.contains("Quoted line"), "plain={plain}");
        assert!(plain.contains("Alpha"), "plain={plain}");
        assert!(plain.contains("Beta"), "plain={plain}");
    }

    #[test]
    fn rst_reader_handles_headings_lists_code_links_and_quotes() {
        let rst = r#"=============
Document Head
=============

Section One
===========

Sub Section
-----------

Tilde Sub
~~~~~~~~~

Minor Heading
^^^^^^^^^^^^^

Quote Heading
"""""""""""""

Paragraph with **bold**, *italic*, ``inline_code()``, `FLIP Site <https://example.com>`_, and `RefLink`_.

.. _RefLink: https://example.org/ref

- First bullet
  continued line
  1. Nested ordered
  2. Second ordered
- Second bullet

Example literal::

    let a = 1;
    let b = 2;

.. code-block:: rust

   fn hello() -> &'static str { "ok" }

Intro before quote:

    Indented blockquote paragraph.

----

+-------+-------+
| Key   | Val   |
+=======+=======+
| lang  | rust  |
+-------+-------+
"#;
        let html = rst_to_html("RST Test", rst);
        assert!(html.contains("<h1>Document Head</h1>"), "html={html}");
        assert!(html.contains("<h2>Section One</h2>"), "html={html}");
        assert!(html.contains("<h3>Sub Section</h3>"), "html={html}");
        assert!(html.contains("<h4>Tilde Sub</h4>"), "html={html}");
        assert!(html.contains("<h5>Minor Heading</h5>"), "html={html}");
        assert!(html.contains("<h6>Quote Heading</h6>"), "html={html}");
        assert!(html.contains("<strong>bold</strong>"), "html={html}");
        assert!(html.contains("<em>italic</em>"), "html={html}");
        assert!(html.contains("<code>inline_code()</code>"), "html={html}");
        assert!(html.contains("<a href=\"https://example.com\">FLIP Site</a>"), "html={html}");
        assert!(html.contains("<a href=\"https://example.org/ref\">RefLink</a>"), "html={html}");
        assert!(html.contains("<ul>"), "html={html}");
        assert!(html.contains("<ol>"), "html={html}");
        assert!(html.contains("<pre><code>let a = 1;\nlet b = 2;</code></pre>"), "html={html}");
        assert!(html.contains("<code class=\"language-rust\">fn hello()"), "html={html}");
        assert!(html.contains("<blockquote>"), "html={html}");
        assert!(html.contains("<hr/>"), "html={html}");
        assert!(html.contains("<table>") && html.contains("<th>Key</th>"), "html={html}");
    }

    #[test]
    fn rst_writer_and_roundtrip_preserve_structure() {
        let md = "# Top Heading\n\n## Sub Heading\n\nText with **bold**, *italic*, ``code``, and [Docs](https://docs.example.com).\n\n- Bullet one\n- Bullet two\n\n> Quoted remark\n\n```\ncode line 1\ncode line 2\n```\n";
        let html = markdown_to_html("RST Roundtrip", md);
        let rst = html_to_rst(&html);

        assert!(rst.contains("Top Heading\n==========="), "rst={rst}");
        assert!(rst.contains("Sub Heading\n-----------"), "rst={rst}");
        assert!(rst.contains("**bold**"), "rst={rst}");
        assert!(rst.contains("*italic*"), "rst={rst}");
        assert!(rst.contains("``code``"), "rst={rst}");
        assert!(rst.contains("`Docs <https://docs.example.com>`_"), "rst={rst}");
        assert!(rst.contains("- Bullet one"), "rst={rst}");
        assert!(rst.contains("::\n\n    code line 1\n    code line 2"), "rst={rst}");

        let roundtrip_html = rst_to_html("RST Roundtrip", &rst);
        let txt = html_to_plain_text(&roundtrip_html);
        assert!(txt.contains("Top Heading"), "txt={txt}");
        assert!(txt.contains("Sub Heading"), "txt={txt}");
        assert!(txt.contains("Bullet one"), "txt={txt}");
        assert!(txt.contains("Quoted remark"), "txt={txt}");
        assert!(txt.contains("code line 1"), "txt={txt}");
    }

    #[test]
    fn rst_reader_handles_utf8_roles_and_escaped_grid_table_pipes() {
        let rst = concat!(
            "Café test: :code:`caffè-perché` **after** and :emphasis:`naïve` *tail*.\n\n",
            "+---------+---------+\n",
            "| Col A   | Col B   |\n",
            "+=========+=========+\n",
            "| x \\| y  | z       |\n",
            "+---------+---------+\n"
        );
        let html = rst_to_html("UTF8", rst);
        assert!(html.contains("<code>caffè-perché</code> <strong>after</strong>"), "html={html}");
        assert!(html.contains("<em>naïve</em> <em>tail</em>"), "html={html}");
        assert!(html.contains("<td>x | y</td><td>z</td>"), "html={html}");
    }

    #[test]
    fn rst_writer_handles_nested_inline_entities_links_svg_and_blockquote_code() {
        let html = concat!(
            "<html><body>",
            "<svg><text>svg-ignored</text></svg>",
            "<p>Literal &amp;lt;tag&amp;gt; and <strong>Hello <em>world</em> and <a href=\"https://ex.com?a=1&amp;b=2\">Docs</a></strong>.</p>",
            "<p><a href=\"https://bare.example/path\"></a> and <a href=\"java&#9;script:alert(1)\">blocked</a></p>",
            "<blockquote><p>Note:</p><pre>let x = 1;\nlet y = 2;</pre></blockquote>",
            "</body></html>"
        );
        let rst = html_to_rst(html);
        assert!(!rst.contains("svg-ignored"), "rst={rst}");
        assert!(rst.contains("Literal &lt;tag&gt;"), "rst={rst}");
        assert!(rst.contains("Hello *world*"), "rst={rst}");
        assert!(rst.contains("`Docs <https://ex.com?a=1&b=2>`_"), "rst={rst}");
        assert!(rst.contains("`https://bare.example/path <https://bare.example/path>`_"), "rst={rst}");
        assert!(!rst.contains("javascript:"), "rst={rst}");
        assert!(rst.contains("blocked"), "rst={rst}");
        assert!(rst.contains("    Note:\n\n    ::\n\n        let x = 1;\n        let y = 2;"), "rst={rst}");

        let back = rst_to_html("Check", &rst);
        assert!(back.contains("<pre><code>let x = 1;\nlet y = 2;</code></pre>"), "back={back}");
    }

    #[test]
    fn fb2_reader_rejects_truncated_and_multiroot_xml() {
        let truncated = "<?xml version=\"1.0\"?><FictionBook><body><section><p>Hi";
        assert!(parse_fb2_to_html(truncated, "T").is_err());

        let multiroot = "<?xml version=\"1.0\"?><FictionBook><body><section><p>Hi</p></section></body></FictionBook><extra/>";
        assert!(parse_fb2_to_html(multiroot, "T").is_err());
    }

    #[test]
    fn fb2_writer_preserves_link_href_across_br_sanitizes_urls_and_handles_table_blocks() {
        let html = concat!(
            "<html><body>",
            "<svg><text>ignore-me</text></svg>",
            "<p><a href=\"https://example.com/?a=1&amp;b=2\">First line<br/>Second line</a></p>",
            "<p><a href=\"java&#9;script:alert(1)\">bad link</a></p>",
            "<table><tr><th><p>Header</p></th></tr><tr><td><ul><li>One</li><li>Two</li></ul><pre>code()</pre><h3>Sub</h3></td></tr></table>",
            "</body></html>"
        );
        let fb2 = write_fb2_from_html("Complex", html);
        assert!(!fb2.contains("ignore-me"), "fb2={fb2}");
        assert!(!fb2.contains("javascript:"), "fb2={fb2}");
        assert!(!fb2.contains("&amp;amp;"), "fb2={fb2}");
        assert!(
            fb2.contains("<p><a l:href=\"https://example.com/?a=1&amp;b=2\">First line</a></p>")
                && fb2.contains("<p><a l:href=\"https://example.com/?a=1&amp;b=2\">Second line</a></p>"),
            "fb2={fb2}"
        );
        assert!(!fb2.contains("<a>Second line</a>"), "fb2={fb2}");
        // Verify strict quick-xml parse_fb2_to_html accepts generated FB2 XML
        let (_title, back_html) = parse_fb2_to_html(&fb2, "Complex").expect("valid FB2 XML");
        assert!(back_html.contains("One"), "back_html={back_html}");
        assert!(back_html.contains("Two"), "back_html={back_html}");
        assert!(back_html.contains("<code>code()</code>"), "back_html={back_html}");
        assert!(back_html.contains("Sub"), "back_html={back_html}");
    }

    #[test]
    fn extract_html_title_strips_inner_tags_decodes_entities_and_collapses_whitespace() {
        let html = r#"<!DOCTYPE html>
<html>
<head>
  <!-- <title>Commented Out</title> -->
  <TITLE data-kind="demo">  3D <span>Snow</span>
  Sphere &amp; Scene  </TITLE >
</head>
</html>"#;
        assert_eq!(
            extract_html_title(html),
            Some("3D Snow Sphere & Scene".to_string())
        );
        assert_eq!(extract_html_title("<html><head><title>   </title></head></html>"), None);
        assert_eq!(extract_html_title("<html><head><title>Unclosed"), None);
        assert_eq!(extract_html_title("<html><body>No title</body></html>"), None);
    }

    #[test]
    fn html_fallbacks_preserve_script_canvas_html_and_do_not_wrap_prose_html() {
        let spa_html = r#"<!DOCTYPE html><html><head><title>3D Snow Sphere &amp; Scene</title><style>canvas{display:block;}</style></head><body><div id="canvas-container"></div><script type="module">console.log("snow");</script></body></html>"#;

        let md = html_to_markdown_with_fallback(spa_html, "snow_sphere");
        assert!(md.starts_with("# 3D Snow Sphere & Scene\n\n```html\n"), "md={md}");
        assert!(md.contains(spa_html), "md={md}");
        assert!(md.ends_with("\n```\n"), "md={md}");

        let txt = html_to_plain_text_with_fallback(spa_html, "snow_sphere");
        assert!(txt.starts_with("3D Snow Sphere & Scene\n\n"), "txt={txt}");
        assert!(txt.contains(spa_html), "txt={txt}");

        let backtick_html = r#"<html><body><script>const s = "```";</script></body></html>"#;
        let fenced_md = html_to_markdown_with_fallback(backtick_html, "fallback_name");
        assert!(fenced_md.starts_with("# fallback_name\n\n~~~~html\n"), "fenced_md={fenced_md}");
        assert!(fenced_md.ends_with("\n~~~~\n"), "fenced_md={fenced_md}");

        assert_eq!(html_to_markdown_with_fallback("   \n\t ", "empty"), "");
        assert_eq!(html_to_plain_text_with_fallback("   \n\t ", "empty"), "");

        let prose_html = "<html><head><title>Ignored</title></head><body><h1>Hello</h1><p>World</p></body></html>";
        let prose_md = html_to_markdown_with_fallback(prose_html, "prose");
        assert_eq!(prose_md, html_to_markdown(prose_html));
        assert!(!prose_md.contains("```html"), "prose_md={prose_md}");
        assert!(!prose_md.contains("Ignored"), "prose_md={prose_md}");

        let prose_txt = html_to_plain_text_with_fallback(prose_html, "prose");
        assert_eq!(prose_txt, html_to_plain_text(prose_html));
        assert!(!prose_txt.contains("<html>"), "prose_txt={prose_txt}");
        assert!(!prose_txt.contains("Ignored"), "prose_txt={prose_txt}");
    }

    #[test]
    fn convert_script_only_html_to_md_and_txt_succeeds_and_passes_validation() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("snow_sphere.html");
        let spa_html = r#"<!DOCTYPE html><html><head><title>3D Snow Sphere &amp; Scene</title><style>canvas{display:block;}</style></head><body><div id="canvas-container"></div><script type="module">console.log("snow");</script></body></html>"#;
        fs::write(&src_path, spa_html).unwrap();

        let prefs = AppPreferences::default();
        let tools = DetectedTools {
            ffmpeg: None,
            ffprobe: None,
            pandoc: None,
            bridge: None,
            pages: false,
            numbers: false,
            keynote: false,
            whisper: None,
            whisper_model: None,
            ghostscript: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));

        let md_stage = tempfile::tempdir().unwrap();
        let md_outputs = convert(&src_path, "MD", md_stage.path(), &prefs, &tools, &cancel, &|_| {})
            .expect("script-only HTML -> MD should succeed");
        assert_eq!(md_outputs.len(), 1);
        crate::engines::validate_output(&md_outputs[0], "MD")
            .expect("MD output should pass validation");
        let md_content = fs::read_to_string(&md_outputs[0]).unwrap();
        assert!(md_content.contains("# 3D Snow Sphere & Scene"), "md={md_content}");
        assert!(md_content.contains("```html"), "md={md_content}");
        assert!(md_content.contains("console.log(\"snow\")"), "md={md_content}");

        let txt_stage = tempfile::tempdir().unwrap();
        let txt_outputs = convert(&src_path, "TXT", txt_stage.path(), &prefs, &tools, &cancel, &|_| {})
            .expect("script-only HTML -> TXT should succeed");
        assert_eq!(txt_outputs.len(), 1);
        crate::engines::validate_output(&txt_outputs[0], "TXT")
            .expect("TXT output should pass validation");
        let txt_content = fs::read_to_string(&txt_outputs[0]).unwrap();
        assert!(txt_content.contains("3D Snow Sphere & Scene"), "txt={txt_content}");
        assert!(txt_content.contains("console.log(\"snow\")"), "txt={txt_content}");
    }
}
