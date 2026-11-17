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

pub fn html_to_markdown(html_input: &str) -> String {
    // Lightweight structured HTML -> Markdown converter when Pandoc is unavailable
    let mut text = html_input.to_string();
    // Strip <head>...</head> and <style>...</style>
    for tag in ["head", "style", "script"] {
        let open_tag = format!("<{tag}");
        let close_tag = format!("</{tag}>");
        while let Some(start) = text.to_ascii_lowercase().find(&open_tag) {
            if let Some(rel_end) = text[start..].to_ascii_lowercase().find(&close_tag) {
                text.replace_range(start..start + rel_end + close_tag.len(), "");
            } else {
                break;
            }
        }
    }

    for (from, to) in [
        ("<h1>", "\n# "),
        ("</h1>", "\n\n"),
        ("<h2>", "\n## "),
        ("</h2>", "\n\n"),
        ("<h3>", "\n### "),
        ("</h3>", "\n\n"),
        ("<h4>", "\n#### "),
        ("</h4>", "\n\n"),
        ("<li>", "\n- "),
        ("</li>", ""),
        ("<strong>", "**"),
        ("</strong>", "**"),
        ("<b>", "**"),
        ("</b>", "**"),
        ("<em>", "*"),
        ("</em>", "*"),
        ("<i>", "*"),
        ("</i>", "*"),
        ("<code>", "`"),
        ("</code>", "`"),
        ("<pre>", "\n```\n"),
        ("</pre>", "\n```\n"),
        ("<br>", "\n"),
        ("<br/>", "\n"),
        ("<br />", "\n"),
        ("</p>", "\n\n"),
        ("</tr>", "\n"),
        ("</td>", " | "),
        ("</th>", " | "),
    ] {
        text = text.replace(from, to);
        text = text.replace(&from.to_ascii_uppercase(), to);
    }

    // Strip remaining tags
    let mut stripped = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }

    let decoded = stripped
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    let lines: Vec<String> = decoded
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    let mut collapsed = String::new();
    let mut blank_run = 0usize;
    for line in lines {
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
    collapsed.trim().to_string() + "\n"
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
