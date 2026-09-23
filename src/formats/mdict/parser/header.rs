use std::collections::HashMap;

use adler32::adler32;
use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
use tracing::info;

pub struct Header {
    pub version: u8,
    /// Encryption bitfield: bit 0 = record blocks, bit 1 = key info block.
    pub encrypted: u8,
    /// Text encoding for MDX key strings, e.g. "UTF-8". Empty → treat as UTF-8.
    pub encoding: String,
    /// Human-readable dictionary title from the `Title` attribute.
    pub title: String,
    /// Dictionary description (may contain HTML).
    pub description: String,
}

/// Parse the MDX/MDD file header.
///
/// Layout: be_u32 length | UTF-16LE XML-ish attrs | le_u32 adler32 checksum.
/// Returns remaining bytes after the header.
pub fn parse_header(data: &[u8]) -> anyhow::Result<(&[u8], Header)> {
    if data.len() < 8 {
        anyhow::bail!(
            "file too small to contain a valid header ({} bytes)",
            data.len()
        );
    }
    let len = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
    if data.len() < 8 + len {
        anyhow::bail!(
            "header extends beyond file (header len={}, file len={})",
            len,
            data.len()
        );
    }
    let buf = &data[4..4 + len];
    let checksum = u32::from_le_bytes(data[4 + len..8 + len].try_into().unwrap());
    if adler32(buf).unwrap() != checksum {
        anyhow::bail!("header adler32 mismatch");
    }

    let xml = UTF_16LE
        .decode(buf, DecoderTrap::Strict)
        .map_err(|e| anyhow::anyhow!("header is not valid UTF-16LE: {e}"))?;

    let attrs = parse_attrs(&xml);
    info!("mdict header attrs: {:?}", attrs);

    let version = attrs
        .get("GeneratedByEngineVersion")
        .and_then(|v| v.trim().chars().next())
        .and_then(|c| c.to_digit(10))
        .unwrap_or(2) as u8;

    let encrypted = attrs
        .get("Encrypted")
        .and_then(|v| v.trim().parse::<u8>().ok())
        .unwrap_or(0);

    let encoding = attrs.get("Encoding").cloned().unwrap_or_default();
    let title = attrs.get("Title").cloned().unwrap_or_default();
    let description = attrs.get("Description").cloned().unwrap_or_default();

    Ok((
        &data[8 + len..],
        Header {
            version,
            encrypted,
            encoding,
            title,
            description,
        },
    ))
}

/// Scan `key="value"` pairs from the MDX XML-ish header string.
pub fn parse_attrs(xml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut s = xml;
    while let Some(eq) = s.find("=\"") {
        let key = s[..eq]
            .split_ascii_whitespace()
            .next_back()
            .unwrap_or("")
            .to_string();
        s = &s[eq + 2..];
        let Some(close) = s.find('"') else { break };
        if !key.is_empty() {
            map.insert(key, s[..close].to_string());
        }
        s = &s[close + 1..];
    }
    map
}

/// Convert the MDX header `StyleSheet` attribute into a CSS string.
///
/// The `StyleSheet` value is a sequence of pairs: a numeric class name
/// on one line, followed by an HTML template on the next.  Dictionary
/// content uses `<span class="1">word</span>` and expects the browser
/// to apply the formatting from the template (colors, bold, italic…).
///
/// We extract the visual properties from each template and emit a
/// `.N { ... }` CSS rule for each numbered class.
pub fn header_stylesheet_to_css(raw: &str) -> String {
    let mut css = String::new();
    let mut lines = raw.split('\n');
    while let Some(class_line) = lines.next() {
        let class = class_line.trim();
        if class.is_empty() {
            continue;
        }
        // Class names in MDX stylesheets are numeric (1, 2, 3…).
        if !class.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Some(template) = lines.next() else { break };

        let mut decls: Vec<String> = Vec::new();

        if template.contains("<b>") || template.contains("<B>") {
            decls.push("font-weight:bold".into());
        }
        if template.contains("<i>") || template.contains("<I>") {
            decls.push("font-style:italic".into());
        }
        if template.contains("<u>") || template.contains("<U>") {
            decls.push("text-decoration:underline".into());
        }
        if let Some(color) = extract_attr(template, "color") {
            decls.push(format!("color:{color}"));
        }
        if let Some(size) = extract_attr(template, "size")
            && let Some(px) = interpret_font_size(&size)
        {
            decls.push(format!("font-size:{px}px"));
        }

        if !decls.is_empty() {
            css.push_str(&format!(".{} {{ {} }}\n", class, decls.join(";")));
        }
    }
    css
}

/// Extract the value of an HTML attribute like `color=red` or `color="#990066"`.
fn extract_attr(html: &str, attr: &str) -> Option<String> {
    let prefix = format!("{attr}=");
    for token in html.split_whitespace() {
        if let Some(val) = token.strip_prefix(&prefix) {
            let val = val.trim_matches('"').trim_matches('\'');
            return Some(val.to_string());
        }
    }
    None
}

/// Convert old-style HTML `<font size=...>` values to pixels.
///
/// Sizes can be: `+N`, `-N`, or absolute 1–7.  The base is 3 → 16 px.
fn interpret_font_size(raw: &str) -> Option<f32> {
    let steps = [10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0]; // HTML 1–7
    if let Some(plus) = raw.strip_prefix('+') {
        let n: i32 = plus.parse().ok()?;
        let idx = (3 + n).clamp(0, 6) as usize;
        Some(steps[idx])
    } else if let Some(minus) = raw.strip_prefix('-') {
        let n: i32 = minus.parse().ok()?;
        let idx = (3 - n).clamp(0, 6) as usize;
        Some(steps[idx])
    } else if let Ok(n) = raw.parse::<usize>() {
        if (1..=7).contains(&n) {
            Some(steps[n - 1])
        } else {
            None
        }
    } else {
        None
    }
}
