use crate::core::policy::ext_lower;
use crate::engines::{bridge, EngineError};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub const MAX_DATA_BYTES: u64 = 20 * 1024 * 1024;

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

    let ext = ext_lower(source);
    let root = match ext.as_str() {
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

fn parse_json(raw: &str) -> Result<Value, EngineError> {
    serde_json::from_str(raw)
        .map_err(|e| EngineError::ConversionFailed(format!("Invalid JSON: {e}")))
}

fn parse_yaml(raw: &str) -> Result<Value, EngineError> {
    serde_yaml::from_str(raw)
        .map_err(|e| EngineError::ConversionFailed(format!("Invalid YAML: {e}")))
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

    let root_start = BytesStart::new("root");
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

fn flatten_object(prefix: &str, obj: &Map<String, Value>, out: &mut Vec<(String, String)>) {
    for (k, v) in obj {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Object(nested) if !nested.is_empty() => flatten_object(&key, nested, out),
            _ => out.push((key, scalar_string(v))),
        }
    }
}

fn to_csv(root: &Value) -> Result<String, EngineError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());

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
                    row_map.insert(k, Value::String(v));
                }
                flat_rows.push(row_map);
            }

            wtr.write_record(&cols)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            for row in flat_rows {
                let fields: Vec<String> = cols
                    .iter()
                    .map(|c| row.get(c).map(scalar_string).unwrap_or_default())
                    .collect();
                wtr.write_record(&fields)
                    .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            }
            let bytes = wtr
                .into_inner()
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
            return String::from_utf8(bytes)
                .map_err(|e| EngineError::ConversionFailed(e.to_string()));
        }
        wtr.write_record(["value"])
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        for item in arr {
            wtr.write_record([scalar_string(item)])
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        }
        let bytes = wtr
            .into_inner()
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        return String::from_utf8(bytes)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()));
    }

    if let Value::Object(map) = root {
        // If root object wraps a primary array of objects (e.g. XML `<root><row>...</row></root>` or TOML `items = [...]`), export it cleanly
        if let Some((_, first_arr)) = map.iter().find(|(_, v)| {
            matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.is_object()))
        }) {
            return to_csv(first_arr);
        }
        wtr.write_record(["key", "value"])
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        let mut pairs = Vec::new();
        flatten_object("", map, &mut pairs);
        for (k, v) in pairs {
            wtr.write_record([k, v])
                .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        }
        let bytes = wtr
            .into_inner()
            .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
        return String::from_utf8(bytes)
            .map_err(|e| EngineError::ConversionFailed(e.to_string()));
    }

    wtr.write_record(["value"])
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    wtr.write_record([scalar_string(root)])
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    let bytes = wtr
        .into_inner()
        .map_err(|e| EngineError::ConversionFailed(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| EngineError::ConversionFailed(e.to_string()))
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
