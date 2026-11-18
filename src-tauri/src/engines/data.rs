use crate::core::policy::{ext_lower, validate_container, MAX_CONTAINER_EXPANDED_BYTES};
use crate::engines::{bridge, EngineError};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const MAX_DATA_BYTES: u64 = 20 * 1024 * 1024;
const MAX_XLSX_COLUMNS: usize = 16_384;
const MAX_XLSX_ROWS: usize = 1_048_576;

pub fn convert(
    source: &Path,
    format: &str,
    output: &Path,
    staging_dir: &Path,
    page_size: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), EngineError> {
    let meta = fs::metadata(source)?;
    if meta.len() > MAX_DATA_BYTES {
        return Err(EngineError::ConversionFailed(
            "Structured data file exceeds 20 MiB limit".to_string(),
        ));
    }

    let ext = ext_lower(source);
    let root = if ext == "xlsx" {
        parse_xlsx(source)?
    } else {
        let raw_bytes = fs::read(source)?;
        let raw_str = String::from_utf8(raw_bytes).map_err(|e| {
            EngineError::ConversionFailed(format!("Data file is not valid UTF-8: {e}"))
        })?;
        let clean_raw = raw_str.strip_prefix('\u{FEFF}').unwrap_or(&raw_str);
        if clean_raw.trim().is_empty() {
            return Err(EngineError::ConversionFailed(
                "Source data file is empty".to_string(),
            ));
        }

        match ext.as_str() {
            "json" => parse_json(clean_raw)?,
            "yaml" | "yml" => parse_yaml(clean_raw)?,
            "toml" => parse_toml(clean_raw)?,
            "xml" => parse_xml(clean_raw)?,
            "csv" => parse_csv(clean_raw)?,
            _ => {
                return Err(EngineError::Unsupported(format!(
                    "Unsupported data extension: .{ext}"
                )))
            }
        }
    };

    let fmt = format.trim().to_ascii_uppercase();
    let title = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("data");

    match fmt.as_str() {
        "JSON" => {
            let s = serde_json::to_string_pretty(&root)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            fs::write(output, s)?;
        }
        "YAML" | "YML" => {
            let s = serde_yaml::to_string(&root)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            fs::write(output, s)?;
        }
        "TOML" => {
            let toml_val = json_to_toml(&root)?;
            let s = toml::to_string_pretty(&toml_val)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            fs::write(output, s)?;
        }
        "XML" => {
            let s = to_xml(&root)?;
            fs::write(output, s)?;
        }
        "CSV" => {
            let s = to_csv(&root)?;
            fs::write(output, s)?;
        }
        "XLSX" => {
            let bytes = to_xlsx(&root)?;
            fs::write(output, bytes)?;
        }
        "TXT" => {
            let s = serde_json::to_string_pretty(&root)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            fs::write(output, s)?;
        }
        "MD" => {
            let s = to_markdown(&root, title)?;
            fs::write(output, s)?;
        }
        "HTML" => {
            let s = to_html(&root, title)?;
            fs::write(output, s)?;
        }
        "PDF" => {
            let readable = serde_json::to_string_pretty(&root)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            let tmp_txt = staging_dir.join("data-render.txt");
            fs::write(&tmp_txt, readable)?;
            bridge::convert_doc(&tmp_txt, "PDF", output, page_size, Some(cancel))?;
        }
        _ => {
            return Err(EngineError::Unsupported(format!(
                "Unsupported data output format: {fmt}"
            )))
        }
    }
    Ok(())
}

fn strip_json_comments_and_trailing_commas(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut no_comments = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < bytes.len() {
        let ch = raw[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        if in_string {
            no_comments.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += ch_len;
            continue;
        }
        if ch == '"' {
            in_string = true;
            no_comments.push(ch);
            i += ch_len;
            continue;
        }
        if ch == '/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
        }
        no_comments.push(ch);
        i += ch_len;
    }

    // Strip trailing commas before `}` or `]` outside strings
    let mut out = String::with_capacity(no_comments.len());
    let chars: Vec<char> = no_comments.chars().collect();
    let mut idx = 0;
    in_string = false;
    escape = false;
    while idx < chars.len() {
        let c = chars[idx];
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            idx += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            idx += 1;
            continue;
        }
        if c == ',' {
            let mut look = idx + 1;
            while look < chars.len() && chars[look].is_whitespace() {
                look += 1;
            }
            if look < chars.len() && (chars[look] == '}' || chars[look] == ']') {
                idx += 1;
                continue;
            }
        }
        out.push(c);
        idx += 1;
    }
    out
}

fn parse_json(raw: &str) -> Result<Value, EngineError> {
    match serde_json::from_str(raw) {
        Ok(val) => Ok(val),
        Err(first_err) => {
            let cleaned = strip_json_comments_and_trailing_commas(raw);
            serde_json::from_str(&cleaned)
                .map_err(|_| EngineError::ConversionFailed(format!("Invalid JSON: {first_err}")))
        }
    }
}

fn yaml_key_to_string(k: &serde_yaml::Value) -> String {
    match k {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn yaml_to_json(val: serde_yaml::Value) -> Value {
    match val {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(Number::from(i))
            } else if let Some(u) = n.as_u64() {
                Value::Number(Number::from(u))
            } else if let Some(f) = n.as_f64() {
                Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::String(n.to_string()))
            } else {
                Value::String(n.to_string())
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(seq) => {
            Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = Map::new();
            for (k, v) in map {
                obj.insert(yaml_key_to_string(&k), yaml_to_json(v));
            }
            Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

fn parse_yaml(raw: &str) -> Result<Value, EngineError> {
    let mut yaml_val: serde_yaml::Value = serde_yaml::from_str(raw)
        .map_err(|e| EngineError::ConversionFailed(format!("Invalid YAML: {e}")))?;
    let _ = yaml_val.apply_merge();
    Ok(yaml_to_json(yaml_val))
}

fn parse_toml(raw: &str) -> Result<Value, EngineError> {
    let val: toml::Value = toml::from_str(raw)
        .map_err(|e| EngineError::ConversionFailed(format!("Invalid TOML: {e}")))?;
    Ok(toml_to_json(val))
}

fn toml_to_json(val: toml::Value) -> Value {
    match val {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(Number::from(i)),
        toml::Value::Float(f) => Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(f.to_string())),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(tbl) => {
            let mut map = Map::new();
            for (k, v) in tbl {
                map.insert(k, toml_to_json(v));
            }
            Value::Object(map)
        }
    }
}

fn json_to_toml_inner(val: &Value) -> Result<toml::Value, EngineError> {
    match val {
        Value::Null => Ok(toml::Value::String("null".to_string())),
        Value::Bool(b) => Ok(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(toml::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(toml::Value::Float(f))
            } else {
                Ok(toml::Value::String(n.to_string()))
            }
        }
        Value::String(s) => Ok(toml::Value::String(s.clone())),
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(json_to_toml_inner(item)?);
            }
            Ok(toml::Value::Array(out))
        }
        Value::Object(map) => {
            let mut tbl = toml::map::Map::new();
            for (k, v) in map {
                tbl.insert(k.clone(), json_to_toml_inner(v)?);
            }
            Ok(toml::Value::Table(tbl))
        }
    }
}

fn json_to_toml(val: &Value) -> Result<toml::Value, EngineError> {
    match val {
        Value::Object(_) => json_to_toml_inner(val),
        Value::Array(_) => {
            let mut root = toml::map::Map::new();
            root.insert("items".to_string(), json_to_toml_inner(val)?);
            Ok(toml::Value::Table(root))
        }
        _ => {
            let mut root = toml::map::Map::new();
            root.insert("value".to_string(), json_to_toml_inner(val)?);
            Ok(toml::Value::Table(root))
        }
    }
}

fn parse_scalar(raw: &str) -> Value {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("null") || v == "~" {
        return Value::Null;
    }
    if v.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if v.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(i) = v.parse::<i64>() {
        return Value::Number(Number::from(i));
    }
    if let Ok(f) = v.parse::<f64>() {
        if f.is_finite() {
            if let Some(n) = Number::from_f64(f) {
                return Value::Number(n);
            }
        }
    }
    Value::String(v.to_string())
}

fn parse_csv(raw: &str) -> Result<Value, EngineError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(raw.as_bytes());
    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let record = rec.map_err(|e| EngineError::ConversionFailed(format!("Invalid CSV: {e}")))?;
        rows.push(record.iter().map(|s| s.to_string()).collect());
    }
    if rows.is_empty() {
        return Err(EngineError::ConversionFailed("CSV is empty".to_string()));
    }

    let headers = &rows[0];
    if rows.len() == 1 {
        let mut single = Map::new();
        for (i, h) in headers.iter().enumerate() {
            single.insert(format!("col_{}", i + 1), Value::String(h.clone()));
        }
        return Ok(Value::Array(vec![Value::Object(single)]));
    }

    let mut out = Vec::new();
    for row in &rows[1..] {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let mut obj = Map::new();
        for (c, hdr) in headers.iter().enumerate() {
            let key = if hdr.trim().is_empty() {
                format!("col_{}", c + 1)
            } else {
                hdr.trim().to_string()
            };
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            obj.insert(key, parse_scalar(cell));
        }
        out.push(Value::Object(obj));
    }
    Ok(Value::Array(out))
}

#[derive(Debug)]
struct XmlNode {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<XmlNode>,
    text: String,
}

fn parse_xml(raw: &str) -> Result<Value, EngineError> {
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
                    let v = a
                        .unescape_value()
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    attrs.push((k, v));
                }
                stack.push(XmlNode {
                    name,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
                    let v = a
                        .unescape_value()
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    attrs.push((k, v));
                }
                let node = XmlNode {
                    name,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    root = Some(node);
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t.unescape().map(|c| c.into_owned()).unwrap_or_default();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&txt);
                }
            }
            Ok(Event::CData(c)) => {
                let txt = String::from_utf8_lossy(&c).into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&txt);
                }
            }
            Ok(Event::End(_)) => {
                if let Some(node) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    } else {
                        root = Some(node);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(EngineError::ConversionFailed(format!("Invalid XML: {e}")));
            }
        }
        buf.clear();
    }

    Ok(root.map(|r| xml_node_to_json(&r)).unwrap_or_else(|| Value::Object(Map::new())))
}

fn xml_node_to_json(node: &XmlNode) -> Value {
    if node.children.is_empty() && node.attrs.is_empty() {
        return parse_scalar(&node.text);
    }

    let mut obj = Map::new();
    for (k, v) in &node.attrs {
        obj.insert(format!("@{k}"), parse_scalar(v));
    }

    let mut grouped: BTreeMap<String, Vec<&XmlNode>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for ch in &node.children {
        if !grouped.contains_key(&ch.name) {
            order.push(ch.name.clone());
        }
        grouped.entry(ch.name.clone()).or_default().push(ch);
    }

    for key in order {
        let items = &grouped[&key];
        if items.len() == 1 {
            obj.insert(key, xml_node_to_json(items[0]));
        } else {
            obj.insert(
                key,
                Value::Array(items.iter().map(|it| xml_node_to_json(it)).collect()),
            );
        }
    }

    if node.children.is_empty() && !node.attrs.is_empty() && !node.text.trim().is_empty() {
        obj.insert("#text".to_string(), parse_scalar(&node.text));
    }

    Value::Object(obj)
}

fn sanitize_xml_name(name: &str) -> String {
    if name.trim().is_empty() {
        return "item".to_string();
    }
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        out.insert(0, '_');
    }
    out
}

fn write_xml_value(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    tag_name: &str,
    val: &Value,
) -> Result<(), EngineError> {
    let clean_tag = sanitize_xml_name(tag_name);
    match val {
        Value::Array(arr) => {
            for item in arr {
                write_xml_value(writer, &clean_tag, item)?;
            }
        }
        Value::Object(map) => {
            let mut start = BytesStart::new(&clean_tag);
            for (k, v) in map {
                if let Some(attr_name) = k.strip_prefix('@') {
                    if let Some(s) = scalar_opt(v) {
                        let attr_clean = sanitize_xml_name(attr_name);
                        start.push_attribute((attr_clean.as_str(), s.as_str()));
                    }
                }
            }
            writer
                .write_event(Event::Start(start))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            for (k, v) in map {
                if k.starts_with('@') {
                    continue;
                }
                if k == "#text" {
                    writer
                        .write_event(Event::Text(BytesText::new(&scalar_string(v))))
                        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
                } else if let Value::Array(arr) = v {
                    for item in arr {
                        write_xml_value(writer, k, item)?;
                    }
                } else {
                    write_xml_value(writer, k, v)?;
                }
            }
            writer
                .write_event(Event::End(BytesEnd::new(&clean_tag)))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        }
        _ => {
            writer
                .write_event(Event::Start(BytesStart::new(&clean_tag)))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            let text = scalar_string(val);
            if !text.is_empty() {
                writer
                    .write_event(Event::Text(BytesText::new(&text)))
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            }
            writer
                .write_event(Event::End(BytesEnd::new(&clean_tag)))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        }
    }
    Ok(())
}

fn to_xml(root: &Value) -> Result<String, EngineError> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    let mut root_start = BytesStart::new("root");
    if let Value::Object(map) = root {
        for (k, v) in map {
            if let Some(attr_name) = k.strip_prefix('@') {
                if let Some(s) = scalar_opt(v) {
                    let attr_clean = sanitize_xml_name(attr_name);
                    root_start.push_attribute((attr_clean.as_str(), s.as_str()));
                }
            }
        }
    }
    writer
        .write_event(Event::Start(root_start))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    match root {
        Value::Array(arr) => {
            for item in arr {
                write_xml_value(&mut writer, "item", item)?;
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                if !k.starts_with('@') {
                    write_xml_value(&mut writer, k, v)?;
                }
            }
        }
        _ => {
            writer
                .write_event(Event::Text(BytesText::new(&scalar_string(root))))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        }
    }

    writer
        .write_event(Event::End(BytesEnd::new("root")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    let bytes = writer.into_inner().into_inner();
    String::from_utf8(bytes).map_err(|e| EngineError::ConversionFailed(e.to_string()))
}

fn scalar_opt(val: &Value) -> Option<String> {
    match val {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn scalar_string(val: &Value) -> String {
    match val {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn flatten_object(prefix: &str, obj: &Map<String, Value>, out: &mut Vec<(String, Value)>) {
    for (k, v) in obj {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Object(nested) if !nested.is_empty() => flatten_object(&key, nested, out),
            _ => out.push((key, v.clone())),
        }
    }
}

fn extract_tabular_rows(root: &Value) -> (Vec<String>, Vec<Vec<Value>>) {
    if let Value::Array(arr) = root {
        if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
            let mut cols: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            let mut flat_rows: Vec<Map<String, Value>> = Vec::with_capacity(arr.len());

            for item in arr {
                let mut pairs = Vec::new();
                if let Value::Object(obj) = item {
                    flatten_object("", obj, &mut pairs);
                }
                let mut row_map = Map::new();
                for (k, v) in pairs {
                    if seen.insert(k.clone()) {
                        cols.push(k.clone());
                    }
                    row_map.insert(k, v);
                }
                flat_rows.push(row_map);
            }

            let rows = flat_rows
                .into_iter()
                .map(|row| {
                    cols.iter()
                        .map(|c| row.get(c).cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .collect();
            return (cols, rows);
        }

        let rows = arr.iter().map(|item| vec![item.clone()]).collect();
        return (vec!["value".to_string()], rows);
    }

    if let Value::Object(map) = root {
        // If root object wraps a primary array of objects (e.g. XML `<root><row>...</row></root>` or TOML `items = [...]`), export it cleanly
        if let Some((_, first_arr)) = map.iter().find(|(_, v)| {
            matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.is_object()))
        }) {
            return extract_tabular_rows(first_arr);
        }
        let mut pairs = Vec::new();
        flatten_object("", map, &mut pairs);
        let rows = pairs
            .into_iter()
            .map(|(k, v)| vec![Value::String(k), v])
            .collect();
        return (vec!["key".to_string(), "value".to_string()], rows);
    }

    (vec!["value".to_string()], vec![vec![root.clone()]])
}

fn to_csv(root: &Value) -> Result<String, EngineError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    let (cols, rows) = extract_tabular_rows(root);
    wtr.write_record(&cols)
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    for row in rows {
        let fields: Vec<String> = row.iter().map(scalar_string).collect();
        wtr.write_record(&fields)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    }
    let bytes = wtr
        .into_inner()
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| EngineError::ConversionFailed(e.to_string()))
}

fn col_index_to_letters(mut col: usize) -> String {
    let mut letters = Vec::new();
    loop {
        let rem = col % 26;
        letters.push((b'A' + rem as u8) as char);
        if col < 26 {
            break;
        }
        col = (col / 26) - 1;
    }
    letters.into_iter().rev().collect()
}

fn sanitize_xml_text(raw: &str) -> String {
    raw.chars()
        .filter(|&c| {
            c == '\t'
                || c == '\n'
                || c == '\r'
                || ('\u{0020}'..='\u{D7FF}').contains(&c)
                || ('\u{E000}'..='\u{FFFD}').contains(&c)
                || c >= '\u{10000}'
        })
        .collect()
}

fn write_xlsx_inline_cell(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    coord: &str,
    raw_text: &str,
) -> Result<(), EngineError> {
    let clean = sanitize_xml_text(raw_text);
    let mut cell = BytesStart::new("c");
    cell.push_attribute(("r", coord));
    cell.push_attribute(("t", "inlineStr"));
    writer
        .write_event(Event::Start(cell))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    writer
        .write_event(Event::Start(BytesStart::new("is")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    let mut t_elem = BytesStart::new("t");
    if clean.starts_with(char::is_whitespace) || clean.ends_with(char::is_whitespace) {
        t_elem.push_attribute(("xml:space", "preserve"));
    }
    writer
        .write_event(Event::Start(t_elem))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    if !clean.is_empty() {
        writer
            .write_event(Event::Text(BytesText::new(&clean)))
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    }
    writer
        .write_event(Event::End(BytesEnd::new("t")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    writer
        .write_event(Event::End(BytesEnd::new("is")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    writer
        .write_event(Event::End(BytesEnd::new("c")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    Ok(())
}

fn write_xlsx_value_cell(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    coord: &str,
    val: &Value,
) -> Result<(), EngineError> {
    match val {
        Value::Null => Ok(()),
        Value::Bool(b) => {
            let mut cell = BytesStart::new("c");
            cell.push_attribute(("r", coord));
            cell.push_attribute(("t", "b"));
            writer
                .write_event(Event::Start(cell))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::Start(BytesStart::new("v")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::Text(BytesText::new(if *b { "1" } else { "0" })))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::End(BytesEnd::new("v")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::End(BytesEnd::new("c")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            Ok(())
        }
        Value::Number(n) if n.as_f64().is_some_and(|f| f.is_finite()) => {
            let mut cell = BytesStart::new("c");
            cell.push_attribute(("r", coord));
            cell.push_attribute(("t", "n"));
            writer
                .write_event(Event::Start(cell))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::Start(BytesStart::new("v")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::Text(BytesText::new(&n.to_string())))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::End(BytesEnd::new("v")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            writer
                .write_event(Event::End(BytesEnd::new("c")))
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            Ok(())
        }
        _ => write_xlsx_inline_cell(writer, coord, &scalar_string(val)),
    }
}

fn to_xlsx(root: &Value) -> Result<Vec<u8>, EngineError> {
    let (cols, rows) = extract_tabular_rows(root);
    if cols.is_empty() {
        return Err(EngineError::ConversionFailed(
            "Cannot export empty dataset to XLSX".to_string(),
        ));
    }
    if cols.len() > MAX_XLSX_COLUMNS {
        return Err(EngineError::ConversionFailed(format!(
            "XLSX column count ({}) exceeds Excel limit ({MAX_XLSX_COLUMNS})",
            cols.len()
        )));
    }
    if rows.len() + 1 > MAX_XLSX_ROWS {
        return Err(EngineError::ConversionFailed(format!(
            "XLSX row count ({}) exceeds Excel limit ({MAX_XLSX_ROWS})",
            rows.len() + 1
        )));
    }

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), Some("yes"))))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    let mut ws = BytesStart::new("worksheet");
    ws.push_attribute((
        "xmlns",
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    ));
    writer
        .write_event(Event::Start(ws))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    writer
        .write_event(Event::Start(BytesStart::new("sheetData")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    // Header row (row 1)
    let mut header_row = BytesStart::new("row");
    header_row.push_attribute(("r", "1"));
    writer
        .write_event(Event::Start(header_row))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    for (col_idx, header) in cols.iter().enumerate() {
        let coord = format!("{}1", col_index_to_letters(col_idx));
        write_xlsx_inline_cell(&mut writer, &coord, header)?;
    }
    writer
        .write_event(Event::End(BytesEnd::new("row")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    // Data rows (row 2..)
    for (r_idx, row) in rows.iter().enumerate() {
        let row_num = r_idx + 2;
        let row_num_str = row_num.to_string();
        let mut row_elem = BytesStart::new("row");
        row_elem.push_attribute(("r", row_num_str.as_str()));
        writer
            .write_event(Event::Start(row_elem))
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        for (col_idx, val) in row.iter().enumerate() {
            if matches!(val, Value::Null) {
                continue;
            }
            let coord = format!("{}{}", col_index_to_letters(col_idx), row_num);
            write_xlsx_value_cell(&mut writer, &coord, val)?;
        }
        writer
            .write_event(Event::End(BytesEnd::new("row")))
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

        if writer.get_ref().get_ref().len() as u64 > MAX_CONTAINER_EXPANDED_BYTES {
            return Err(EngineError::ConversionFailed(
                "Generated XLSX worksheet exceeds 256 MiB safety limit".to_string(),
            ));
        }
    }

    writer
        .write_event(Event::End(BytesEnd::new("sheetData")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    writer
        .write_event(Event::End(BytesEnd::new("worksheet")))
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;

    let sheet1 = writer.into_inner().into_inner();

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
    let styles = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;

    let entries: [(&str, &[u8]); 6] = [
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", rels),
        ("xl/_rels/workbook.xml.rels", wb_rels),
        ("xl/workbook.xml", workbook),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", &sheet1),
    ];

    let total_uncompressed: u64 = entries.iter().map(|(_, b)| b.len() as u64).sum();
    if total_uncompressed > MAX_CONTAINER_EXPANDED_BYTES {
        return Err(EngineError::ConversionFailed(
            "Generated XLSX archive exceeds 256 MiB safety limit".to_string(),
        ));
    }

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zip.start_file(name, opts)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        zip.write_all(bytes)?;
    }
    let finished = zip
        .finish()
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    Ok(finished.into_inner())
}

fn read_zip_entry_string(archive: &mut ZipArchive<File>, target: &str) -> Option<String> {
    let normalized = target.trim_start_matches('/');
    if let Ok(mut entry) = archive.by_name(normalized) {
        let mut s = String::new();
        if entry.read_to_string(&mut s).is_ok() {
            return Some(s);
        }
    }
    let matched_name = (0..archive.len()).find_map(|i| {
        archive.by_index(i).ok().and_then(|e| {
            let n = e.name().trim_start_matches('/');
            if n.eq_ignore_ascii_case(normalized) {
                Some(e.name().to_string())
            } else {
                None
            }
        })
    })?;
    let mut entry = archive.by_name(&matched_name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

fn normalize_ooxml_target(base_dir: &str, target: &str) -> String {
    let raw = target.trim();
    let combined = if let Some(abs) = raw.strip_prefix('/') {
        abs.to_string()
    } else if base_dir.is_empty() {
        raw.to_string()
    } else {
        format!("{base_dir}/{raw}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn resolve_first_worksheet_path(archive: &mut ZipArchive<File>) -> String {
    if let Some(wb_xml) = read_zip_entry_string(archive, "xl/workbook.xml") {
        let mut reader = Reader::from_str(&wb_xml);
        let mut buf = Vec::new();
        let mut first_rid: Option<String> = None;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if e.local_name().as_ref() == b"sheet" {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"r:id" || attr.key.local_name().as_ref() == b"id"
                            {
                                if let Ok(v) = attr.unescape_value() {
                                    first_rid = Some(v.into_owned());
                                    break;
                                }
                            }
                        }
                        if first_rid.is_some() {
                            break;
                        }
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        if let Some(rid) = first_rid {
            if let Some(rels_xml) = read_zip_entry_string(archive, "xl/_rels/workbook.xml.rels") {
                let mut rel_reader = Reader::from_str(&rels_xml);
                let mut rel_buf = Vec::new();
                loop {
                    match rel_reader.read_event_into(&mut rel_buf) {
                        Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                            if e.local_name().as_ref() == b"Relationship" {
                                let mut id_val = None;
                                let mut target_val = None;
                                for attr in e.attributes().flatten() {
                                    let k = attr.key.local_name();
                                    if k.as_ref() == b"Id" {
                                        id_val = attr.unescape_value().ok().map(|v| v.into_owned());
                                    } else if k.as_ref() == b"Target" {
                                        target_val =
                                            attr.unescape_value().ok().map(|v| v.into_owned());
                                    }
                                }
                                if id_val.as_deref() == Some(rid.as_str()) {
                                    if let Some(t) = target_val {
                                        return normalize_ooxml_target("xl", &t);
                                    }
                                }
                            }
                        }
                        Ok(Event::Eof) | Err(_) => break,
                        _ => {}
                    }
                    rel_buf.clear();
                }
            }
        }
    }

    if archive.by_name("xl/worksheets/sheet1.xml").is_ok() {
        return "xl/worksheets/sheet1.xml".to_string();
    }

    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name();
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("xl/worksheets/") && lower.ends_with(".xml") {
                return name.to_string();
            }
        }
    }

    "xl/worksheets/sheet1.xml".to_string()
}

fn parse_shared_strings(xml: &str) -> Result<Vec<String>, EngineError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut strings = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut rph_depth: usize = 0;
    let mut current = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"si" => {
                    in_si = true;
                    in_t = false;
                    rph_depth = 0;
                    current.clear();
                }
                b"rPh" if in_si => {
                    rph_depth += 1;
                }
                b"t" if in_si && rph_depth == 0 => {
                    in_t = true;
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"si" {
                    strings.push(String::new());
                }
            }
            Ok(Event::Text(t)) if in_si && in_t && rph_depth == 0 => {
                let txt = t
                    .unescape()
                    .map_err(|e| EngineError::ConversionFailed(format!("Invalid sharedStrings XML: {e}")))?;
                current.push_str(&txt);
            }
            Ok(Event::CData(c)) if in_si && in_t && rph_depth == 0 => {
                current.push_str(&String::from_utf8_lossy(&c));
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_t = false,
                b"rPh" => rph_depth = rph_depth.saturating_sub(1),
                b"si" => {
                    in_si = false;
                    in_t = false;
                    rph_depth = 0;
                    strings.push(std::mem::take(&mut current));
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(EngineError::ConversionFailed(format!(
                    "Invalid XLSX sharedStrings XML: {e}"
                )));
            }
        }
        buf.clear();
    }

    Ok(strings)
}

fn parse_cell_col_index(coord: &str) -> Option<usize> {
    let mut col: usize = 0;
    let mut saw_alpha = false;
    for b in coord.bytes() {
        if b.is_ascii_alphabetic() {
            saw_alpha = true;
            let digit = (b.to_ascii_uppercase() - b'A') as usize + 1;
            col = col.checked_mul(26)?.checked_add(digit)?;
        } else {
            break;
        }
    }
    if saw_alpha && col > 0 {
        Some(col - 1)
    } else {
        None
    }
}

fn parse_xlsx_number_or_scalar(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Value::Number(Number::from(i));
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        if f.is_finite() {
            if let Some(n) = Number::from_f64(f) {
                return Value::Number(n);
            }
        }
    }
    parse_scalar(trimmed)
}

fn is_cell_non_empty(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        _ => true,
    }
}

fn parse_xlsx(path: &Path) -> Result<Value, EngineError> {
    validate_container(path)?;

    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| EngineError::ConversionFailed(format!("Invalid XLSX archive: {e}")))?;

    let shared_strings = if let Some(ss_xml) = read_zip_entry_string(&mut archive, "xl/sharedStrings.xml") {
        parse_shared_strings(&ss_xml)?
    } else {
        Vec::new()
    };

    let sheet_path = resolve_first_worksheet_path(&mut archive);
    let sheet_xml = read_zip_entry_string(&mut archive, &sheet_path).ok_or_else(|| {
        EngineError::ConversionFailed(format!("XLSX worksheet not found: {sheet_path}"))
    })?;

    let mut reader = Reader::from_str(&sheet_xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut rows: Vec<BTreeMap<usize, Value>> = Vec::new();
    let mut max_col: usize = 0;

    let mut in_row = false;
    let mut current_row: BTreeMap<usize, Value> = BTreeMap::new();
    let mut next_col_idx: usize = 0;

    let mut in_cell = false;
    let mut cell_col: usize = 0;
    let mut cell_type = String::new();
    let mut in_v = false;
    let mut in_is = false;
    let mut in_t = false;
    let mut rph_depth: usize = 0;
    let mut has_value = false;
    let mut cell_buf = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"row" => {
                    in_row = true;
                    current_row.clear();
                    next_col_idx = 0;
                }
                b"c" if in_row => {
                    in_cell = true;
                    cell_type.clear();
                    cell_buf.clear();
                    has_value = false;
                    in_v = false;
                    in_is = false;
                    in_t = false;
                    rph_depth = 0;

                    let mut parsed_col = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.local_name().as_ref() {
                            b"r" => {
                                if let Ok(v) = attr.unescape_value() {
                                    parsed_col = parse_cell_col_index(&v);
                                }
                            }
                            b"t" => {
                                if let Ok(v) = attr.unescape_value() {
                                    cell_type = v.into_owned();
                                }
                            }
                            _ => {}
                        }
                    }
                    cell_col = parsed_col.unwrap_or(next_col_idx);
                    if cell_col >= MAX_XLSX_COLUMNS {
                        return Err(EngineError::ConversionFailed(
                            "XLSX cell column coordinate exceeds Excel limit".to_string(),
                        ));
                    }
                    next_col_idx = cell_col + 1;
                }
                b"v" if in_cell => {
                    in_v = true;
                    has_value = true;
                }
                b"is" if in_cell => {
                    in_is = true;
                }
                b"rPh" if in_cell && in_is => {
                    rph_depth += 1;
                }
                b"t" if in_cell && in_is && rph_depth == 0 => {
                    in_t = true;
                    has_value = true;
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"c" && in_row {
                    let mut parsed_col = None;
                    for attr in e.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"r" {
                            if let Ok(v) = attr.unescape_value() {
                                parsed_col = parse_cell_col_index(&v);
                            }
                        }
                    }
                    let col = parsed_col.unwrap_or(next_col_idx);
                    if col >= MAX_XLSX_COLUMNS {
                        return Err(EngineError::ConversionFailed(
                            "XLSX cell column coordinate exceeds Excel limit".to_string(),
                        ));
                    }
                    next_col_idx = col + 1;
                }
            }
            Ok(Event::Text(t)) if in_cell && (in_v || (in_is && in_t && rph_depth == 0)) => {
                let txt = t
                    .unescape()
                    .map_err(|e| EngineError::ConversionFailed(format!("Invalid XLSX XML: {e}")))?;
                cell_buf.push_str(&txt);
            }
            Ok(Event::CData(c)) if in_cell && (in_v || (in_is && in_t && rph_depth == 0)) => {
                cell_buf.push_str(&String::from_utf8_lossy(&c));
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"v" => in_v = false,
                b"t" => in_t = false,
                b"rPh" => rph_depth = rph_depth.saturating_sub(1),
                b"is" => {
                    in_is = false;
                    in_t = false;
                    rph_depth = 0;
                }
                b"c" if in_cell => {
                    in_cell = false;
                    if has_value {
                        let cell_val = match cell_type.as_str() {
                            "s" => {
                                let idx: usize = cell_buf.trim().parse().map_err(|_| {
                                    EngineError::ConversionFailed(format!(
                                        "Invalid XLSX shared string index: `{cell_buf}`"
                                    ))
                                })?;
                                let s = shared_strings.get(idx).cloned().ok_or_else(|| {
                                    EngineError::ConversionFailed(format!(
                                        "XLSX shared string index {idx} out of bounds"
                                    ))
                                })?;
                                Value::String(s)
                            }
                            "inlineStr" | "str" | "e" | "d" => Value::String(std::mem::take(&mut cell_buf)),
                            "b" => match cell_buf.trim() {
                                "1" | "true" | "TRUE" => Value::Bool(true),
                                "0" | "false" | "FALSE" => Value::Bool(false),
                                other => parse_scalar(other),
                            },
                            "" | "n" => {
                                if cell_buf.trim().is_empty() {
                                    Value::Null
                                } else {
                                    parse_xlsx_number_or_scalar(&cell_buf)
                                }
                            }
                            _ => parse_scalar(&cell_buf),
                        };
                        if is_cell_non_empty(&cell_val) {
                            current_row.insert(cell_col, cell_val);
                        }
                    }
                    cell_buf.clear();
                }
                b"row" if in_row => {
                    in_row = false;
                    if current_row.values().any(is_cell_non_empty) {
                        if let Some(&last_col) = current_row.keys().next_back() {
                            max_col = max_col.max(last_col);
                        }
                        rows.push(std::mem::take(&mut current_row));
                    } else {
                        current_row.clear();
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(EngineError::ConversionFailed(format!(
                    "Invalid XLSX worksheet XML: {e}"
                )));
            }
        }
        buf.clear();
    }

    if rows.is_empty() {
        return Err(EngineError::ConversionFailed(
            "XLSX worksheet is empty or has no data".to_string(),
        ));
    }

    let num_cols = max_col + 1;
    if rows.len() == 1 {
        let mut single = Map::new();
        for c in 0..num_cols {
            let val = rows[0]
                .get(&c)
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            single.insert(format!("col_{}", c + 1), val);
        }
        return Ok(Value::Array(vec![Value::Object(single)]));
    }

    let headers: Vec<String> = (0..num_cols)
        .map(|c| {
            let hdr = rows[0].get(&c).map(scalar_string).unwrap_or_default();
            if hdr.trim().is_empty() {
                format!("col_{}", c + 1)
            } else {
                hdr.trim().to_string()
            }
        })
        .collect();

    let mut out = Vec::with_capacity(rows.len() - 1);
    for row in &rows[1..] {
        let mut obj = Map::new();
        for (c, key) in headers.iter().enumerate() {
            let val = row
                .get(&c)
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            obj.insert(key.clone(), val);
        }
        out.push(Value::Object(obj));
    }
    Ok(Value::Array(out))
}

fn to_markdown(root: &Value, title: &str) -> Result<String, EngineError> {
    let mut out = format!("# {title}\n\n");
    if let Value::Array(arr) = root {
        if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
            let mut cols = Vec::new();
            let mut seen = HashSet::new();
            for item in arr {
                if let Value::Object(obj) = item {
                    for k in obj.keys() {
                        if seen.insert(k.clone()) {
                            cols.push(k.clone());
                        }
                    }
                }
            }
            out.push_str(&format!("| {} |\n", cols.join(" | ")));
            let sep: Vec<&str> = cols.iter().map(|_| "---").collect();
            out.push_str(&format!("| {} |\n", sep.join(" | ")));
            for item in arr {
                if let Value::Object(obj) = item {
                    let row: Vec<String> = cols
                        .iter()
                        .map(|c| {
                            obj.get(c)
                                .map(scalar_string)
                                .unwrap_or_default()
                                .replace('|', "\\|")
                                .replace('\n', " ")
                        })
                        .collect();
                    out.push_str(&format!("| {} |\n", row.join(" | ")));
                }
            }
            return Ok(out);
        }
    }
    let pretty = serde_json::to_string_pretty(root)
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    out.push_str("```json\n");
    out.push_str(&pretty);
    out.push_str("\n```\n");
    Ok(out)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn to_html(root: &Value, title: &str) -> Result<String, EngineError> {
    let mut out = String::new();
    out.push_str("<!doctype html><html><head><meta charset=\"utf-8\">");
    out.push_str(&format!("<title>{}</title>", html_escape(title)));
    out.push_str("<style>body{font-family:-apple-system,BlinkMacSystemFont,sans-serif;background:#16181c;color:#f1f3f5;padding:28px;margin:0}h1{font-size:20px;margin-bottom:16px;color:#ff8c24}table{border-collapse:collapse;width:100%;background:#22252a;border-radius:8px;overflow:hidden}th,td{border:1px solid #363a42;padding:10px 12px;text-align:left;font-size:14px}th{background:#2c3036;color:#ff8c24}pre{background:#22252a;padding:18px;border-radius:8px;overflow:auto;border:1px solid #363a42;font-family:ui-monospace,SFMono-Regular,monospace;font-size:13px;line-height:1.5}</style>");
    out.push_str(&format!("</head><body><h1>{}</h1>", html_escape(title)));

    if let Value::Array(arr) = root {
        if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
            let mut cols = Vec::new();
            let mut seen = HashSet::new();
            for item in arr {
                if let Value::Object(obj) = item {
                    for k in obj.keys() {
                        if seen.insert(k.clone()) {
                            cols.push(k.clone());
                        }
                    }
                }
            }
            out.push_str("<table><thead><tr>");
            for c in &cols {
                out.push_str(&format!("<th>{}</th>", html_escape(c)));
            }
            out.push_str("</tr></thead><tbody>");
            for item in arr {
                if let Value::Object(obj) = item {
                    out.push_str("<tr>");
                    for c in &cols {
                        let val = obj.get(c).map(scalar_string).unwrap_or_default();
                        out.push_str(&format!("<td>{}</td>", html_escape(&val)));
                    }
                    out.push_str("</tr>");
                }
            }
            out.push_str("</tbody></table></body></html>");
            return Ok(out);
        }
    }

    let pretty = serde_json::to_string_pretty(root)
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    out.push_str("<pre>");
    out.push_str(&html_escape(&pretty));
    out.push_str("</pre></body></html>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn yaml_non_string_keys_anchors_and_json_comments_trailing_commas() {
        let yaml_raw = "1: one\ntrue: yes\nbase: &base\n  role: admin\nuser:\n  <<: *base\n  name: \"Ada\"\n";
        let parsed = parse_yaml(yaml_raw).unwrap();
        assert_eq!(parsed["1"], Value::String("one".into()));
        assert_eq!(parsed["true"], Value::String("yes".into()));
        assert_eq!(parsed["user"]["role"], Value::String("admin".into()));

        let json_commented = "{\n  // comment\n  \"a\": 1, /* block */\n  \"b\": [1, 2,],\n}\n";
        let parsed_j = parse_json(json_commented).unwrap();
        assert_eq!(parsed_j["a"], Value::Number(Number::from(1)));
        assert_eq!(parsed_j["b"].as_array().unwrap().len(), 2);

        let dir = tempdir().unwrap();
        let xml_src = dir.path().join("attr.xml");
        let json_out = dir.path().join("attr.json");
        let xml_back = dir.path().join("attr_back.xml");
        let cancel = Arc::new(AtomicBool::new(false));
        fs::write(&xml_src, r#"<root version="2.0"><item id="9">hello</item></root>"#).unwrap();
        convert(&xml_src, "JSON", &json_out, dir.path(), "a4", &cancel).unwrap();
        convert(&json_out, "XML", &xml_back, dir.path(), "a4", &cancel).unwrap();
        let back_str = fs::read_to_string(&xml_back).unwrap();
        assert!(back_str.contains(r#"version="2.0""#), "xml lost root attr: {back_str}");
    }

    #[test]
    fn xlsx_column_coords_shared_strings_and_edge_cases() {
        assert_eq!(col_index_to_letters(0), "A");
        assert_eq!(col_index_to_letters(25), "Z");
        assert_eq!(col_index_to_letters(26), "AA");
        assert_eq!(col_index_to_letters(51), "AZ");
        assert_eq!(col_index_to_letters(52), "BA");
        assert_eq!(col_index_to_letters(702), "AAA");

        assert_eq!(parse_cell_col_index("A1"), Some(0));
        assert_eq!(parse_cell_col_index("Z99"), Some(25));
        assert_eq!(parse_cell_col_index("AA12"), Some(26));
        assert_eq!(parse_cell_col_index("AZ5"), Some(51));
        assert_eq!(parse_cell_col_index("BA1"), Some(52));
        assert_eq!(parse_cell_col_index("123"), None);

        let ss_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3" uniqueCount="3">
  <si><t>Name</t></si>
  <si><r><t xml:space="preserve">Rich </t></r><r><t>Text</t></r><rPh><t>ignored</t></rPh></si>
  <si/>
</sst>"#;
        let ss = parse_shared_strings(ss_xml).unwrap();
        assert_eq!(ss, vec!["Name", "Rich Text", ""]);
    }
}
