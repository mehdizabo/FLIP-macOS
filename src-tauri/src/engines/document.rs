use crate::core::naming::format_extension;
use crate::core::policy::ext_lower;
use crate::core::preferences::AppPreferences;
use crate::engines::{bridge, pdf, DetectedTools, EngineError};
use pulldown_cmark::{html, Options, Parser};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub const MAX_TEXT_READ_BYTES: u64 = 2 * 1024 * 1024;

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

fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi_rel) = s[i..].find(';') {
                if semi_rel <= 12 {
                    let entity = &s[i + 1..i + semi_rel];
                    let decoded = match entity {
                        "nbsp" => Some(" ".to_string()),
                        "amp" => Some("&".to_string()),
                        "lt" => Some("<".to_string()),
                        "gt" => Some(">".to_string()),
                        "quot" => Some("\"".to_string()),
                        "apos" | "#39" => Some("'".to_string()),
                        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                            u32::from_str_radix(&entity[2..], 16)
                                .ok()
                                .and_then(char::from_u32)
                                .map(|c| c.to_string())
                        }
                        _ if entity.starts_with('#') => entity[1..]
                            .parse::<u32>()
                            .ok()
                            .and_then(char::from_u32)
                            .map(|c| c.to_string()),
                        _ => None,
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

fn extract_attr(tag_inner: &str, attr_name: &str) -> Option<String> {
    let lower = tag_inner.to_ascii_lowercase();
    let needle = format!("{attr_name}=");
    let pos = lower.find(&needle)?;
    let rest = tag_inner[pos + needle.len()..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().map(String::from)
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        stripped.split('\'').next().map(String::from)
    } else {
        rest.split_whitespace().next().map(String::from)
    }
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
                out.push_str(&html_input[i..]);
                break;
            }

            let raw_tag = html_input[i + 1..j].trim();
            i = j + 1;

            if raw_tag.starts_with("!--") || raw_tag.starts_with('!') || raw_tag.starts_with('?') {
                continue;
            }

            let is_close = raw_tag.starts_with('/');
            let inner = if is_close { raw_tag[1..].trim() } else { raw_tag };
            let tag_name = inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            if let Some(ref skipping) = skip_tag {
                if is_close && &tag_name == skipping {
                    skip_tag = None;
                }
                continue;
            }

            if !is_close && matches!(tag_name.as_str(), "head" | "style" | "script") && !inner.ends_with('/') {
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

    let decoded = decode_html_entities(&out);
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

    unit_progress(25);

    // 1) Apple iWork (.pages, .numbers, .key)
    if matches!(ext.as_str(), "pages" | "numbers" | "key") {
        bridge::convert_doc(source, &fmt, &output, &prefs.pdf_page_size, Some(cancel))?;
        return Ok(vec![output]);
    }

    // 2) Render document to images via intermediate PDF
    if matches!(fmt.as_str(), "PNG" | "JPG" | "WEBP") {
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
        let title = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Document");
        let html_str = markdown_to_html(title, &md_text);
        let html_path = staging_dir.join("markdown.html");
        fs::write(&html_path, html_str)?;
        if fmt == "HTML" {
            return Ok(vec![html_path]);
        }
        prepared_input = html_path;
        prepared_ext = "html".to_string();
    } else {
        prepared_input = source.to_path_buf();
        prepared_ext = ext.clone();
    }

    // 4) Pandoc formats (EPUB, FB2, RST) either as source or target
    if matches!(prepared_ext.as_str(), "epub" | "fb2" | "rst") || matches!(fmt.as_str(), "EPUB" | "FB2" | "RST") {
        let from_pandoc = match prepared_ext.as_str() {
            "md" | "markdown" => "markdown",
            "htm" | "html" => "html",
            "txt" => "plain",
            other => other,
        };
        if matches!(fmt.as_str(), "EPUB" | "FB2" | "RST") && prepared_ext == "txt" {
            let txt_raw = fs::read_to_string(&prepared_input)?;
            let html_path = staging_dir.join("txt-wrap.html");
            fs::write(&html_path, plain_text_to_html("FLIP Document", &txt_raw))?;
            convert_via_pandoc(&html_path, "html", &fmt, &output, staging_dir, tools, cancel)?;
            return Ok(vec![output]);
        }
        if matches!(prepared_ext.as_str(), "epub" | "fb2" | "rst") && fmt == "PDF" {
            let html_mid = staging_dir.join("pandoc-mid.html");
            convert_via_pandoc(&prepared_input, from_pandoc, "HTML", &html_mid, staging_dir, tools, cancel)?;
            bridge::convert_doc(&html_mid, "PDF", &output, &prefs.pdf_page_size, Some(cancel))?;
            return Ok(vec![output]);
        }
        convert_via_pandoc(&prepared_input, from_pandoc, &fmt, &output, staging_dir, tools, cancel)?;
        return Ok(vec![output]);
    }

    // 5) Target is MD
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

    // 6) Native AppKit / NSAttributedString / CoreText bridge (TXT, HTML, RTF, DOC, DOCX, ODT -> PDF/DOCX/RTF/HTML/TXT/ODT)
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
}
