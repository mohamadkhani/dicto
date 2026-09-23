pub mod parser;

use std::fs::{self, File};
#[cfg(not(unix))]
use std::io::Read;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use fst::automaton::{AlwaysMatch, Levenshtein, Str as FstStr};
use fst::{Automaton, IntoStreamer, Map, MapBuilder, Streamer};
use memmap2::Mmap;
use tracing::{info, warn};

use crate::dictionary::Dictionary;
use parser::header::{header_stylesheet_to_css, parse_attrs};
use parser::mdd::{Mdd, normalize_path};
use parser::mdx::Mdx;
use parser::recordblock::decompress_record_block;

// ── offset record (24 bytes per entry) ───────────────────────────────────────

/// Packed encoding of where one entry lives in the source file.
///
/// Layout (little-endian):
///   [0..8]   file_offset    u64 — byte offset of compressed block in source file
///   [8..12]  block_csize    u32
///   [12..16] block_dsize    u32
///   [16..20] start_in_block u32
///   [20..24] end_in_block   u32
const REC_LEN: usize = 24;

fn encode_offset(file_offset: u64, csize: u32, dsize: u32, start: u32, end: u32) -> [u8; REC_LEN] {
    let mut b = [0u8; REC_LEN];
    b[0..8].copy_from_slice(&file_offset.to_le_bytes());
    b[8..12].copy_from_slice(&csize.to_le_bytes());
    b[12..16].copy_from_slice(&dsize.to_le_bytes());
    b[16..20].copy_from_slice(&start.to_le_bytes());
    b[20..24].copy_from_slice(&end.to_le_bytes());
    b
}

fn decode_offset(b: &[u8]) -> Option<(u64, u32, u32, u32, u32)> {
    if b.len() < REC_LEN {
        return None;
    }
    let file_offset = u64::from_le_bytes(b[0..8].try_into().unwrap());
    let csize = u32::from_le_bytes(b[8..12].try_into().unwrap());
    let dsize = u32::from_le_bytes(b[12..16].try_into().unwrap());
    let start = u32::from_le_bytes(b[16..20].try_into().unwrap());
    let end = u32::from_le_bytes(b[20..24].try_into().unwrap());
    Some((file_offset, csize, dsize, start, end))
}

const MAX_REDIRECTS: u8 = 5;

// ── FST index ─────────────────────────────────────────────────────────────────

struct FstIndex {
    map: Map<Mmap>,
    offsets: Mmap,
}

impl FstIndex {
    fn open(fst_path: &str, off_path: &str) -> Option<Arc<Self>> {
        let fst_mmap = unsafe { Mmap::map(&File::open(fst_path).ok()?).ok()? };
        let map = Map::new(fst_mmap).ok()?;
        let offsets = unsafe { Mmap::map(&File::open(off_path).ok()?).ok()? };
        Some(Arc::new(Self { map, offsets }))
    }

    fn get_record(&self, key: &str) -> Option<&[u8]> {
        let row = self.map.get(key.as_bytes())? as usize;
        let start = row * REC_LEN;
        self.offsets.get(start..start + REC_LEN)
    }
}

// ── file read helper (pread — no cursor, thread-safe) ─────────────────────────

#[cfg(unix)]
fn read_block_at(file: &File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let mut buf = vec![0u8; len];
    file.read_at(&mut buf, offset)?;
    Ok(buf)
}

#[cfg(not(unix))]
fn read_block_at(file: &File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Seek, SeekFrom};
    let mut f = file.try_clone()?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_entry(
    file: &File,
    file_offset: u64,
    csize: u32,
    dsize: u32,
    start: u32,
    end: u32,
) -> Option<Vec<u8>> {
    let block = read_block_at(file, file_offset, csize as usize).ok()?;
    let decompressed = decompress_record_block(&block, csize as usize, dsize as usize).ok()?;
    let end = (end as usize).min(decompressed.len());
    let start = (start as usize).min(end);
    Some(decompressed[start..end].to_vec())
}

// ── header stylesheet extraction ─────────────────────────────────────────────

/// Read the MDX header and return the CSS derived from its `StyleSheet` attribute.
/// Returns an empty string if the file cannot be read or has no stylesheet.
pub fn mdx_header_stylesheet(mdx_path: &str) -> String {
    let Ok(data) = std::fs::read(mdx_path) else {
        return String::new();
    };
    let len = match data.get(0..4) {
        Some(b) => u32::from_be_bytes(b.try_into().unwrap()) as usize,
        None => return String::new(),
    };
    let buf = match data.get(4..4 + len) {
        Some(b) => b,
        None => return String::new(),
    };
    use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
    let xml = match UTF_16LE.decode(buf, DecoderTrap::Strict) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let attrs = parse_attrs(&xml);
    match attrs.get("StyleSheet") {
        Some(raw) if !raw.trim().is_empty() => header_stylesheet_to_css(raw),
        _ => String::new(),
    }
}

/// Read the MDX header and return the `Title` attribute.
/// Returns the file stem if the file cannot be read or has no Title.
pub fn mdx_header_title(mdx_path: &str) -> String {
    let stem = PathBuf::from(mdx_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(mdx_path)
        .to_string();

    let Ok(data) = std::fs::read(mdx_path) else {
        return stem;
    };
    let len = match data.get(0..4) {
        Some(b) => u32::from_be_bytes(b.try_into().unwrap()) as usize,
        None => return stem,
    };
    let buf = match data.get(4..4 + len) {
        Some(b) => b,
        None => return stem,
    };
    use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
    let xml = match UTF_16LE.decode(buf, DecoderTrap::Strict) {
        Ok(s) => s,
        Err(_) => return stem,
    };
    let attrs = parse_attrs(&xml);
    attrs
        .get("Title")
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .unwrap_or(stem)
}

/// Lightweight header metadata (no full Dictionary instance needed).
pub struct HeaderMeta {
    pub encoding: String,
    pub version: u8,
    pub description: String,
}

/// Read encoding, version, description from the MDX header.
pub fn mdx_header_info(mdx_path: &str) -> HeaderMeta {
    let default = HeaderMeta {
        encoding: String::new(),
        version: 0,
        description: String::new(),
    };
    let Ok(data) = std::fs::read(mdx_path) else {
        return default;
    };
    let len = match data.get(0..4) {
        Some(b) => u32::from_be_bytes(b.try_into().unwrap()) as usize,
        None => return default,
    };
    let buf = match data.get(4..4 + len) {
        Some(b) => b,
        None => return default,
    };
    use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
    let xml = match UTF_16LE.decode(buf, DecoderTrap::Strict) {
        Ok(s) => s,
        Err(_) => return default,
    };
    let attrs = parse_attrs(&xml);
    let description = attrs.get("Description").cloned().unwrap_or_default();
    HeaderMeta {
        encoding: attrs.get("Encoding").cloned().unwrap_or_default(),
        version: attrs
            .get("GeneratedByEngineVersion")
            .and_then(|v| v.trim().chars().next())
            .and_then(|c| c.to_digit(10))
            .unwrap_or(0) as u8,
        description: clean_description(&description),
    }
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Strip HTML tags, <style> blocks, and decode common HTML entities.
/// Truncate to 300 chars.
fn clean_description(raw: &str) -> String {
    // Remove <style...>...</style> blocks first (handles attributes like <style type="text/css">).
    let mut out = String::with_capacity(raw.len());
    let mut skip = false;
    let chars = raw.chars().collect::<Vec<_>>();
    let lower: String = raw.to_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut i = 0;
    while i < chars.len() {
        // Check for <style opening tag (may have attributes)
        if lower_bytes.get(i).copied() == Some(b'<')
            && lower_bytes
                .get(i + 1..i + 6)
                .map(|s| s == b"style")
                .unwrap_or(false)
            && lower_bytes
                .get(i + 6)
                .copied()
                .map(|c| c == b'>' || c == b' ')
                .unwrap_or(false)
        {
            skip = true;
            // Skip to the end of the opening tag
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            i += 1; // skip the '>'
            continue;
        }
        // Check for </style>
        if lower_bytes
            .get(i..i + 8)
            .map(|s| s == b"</style>")
            .unwrap_or(false)
        {
            skip = false;
            i += 8;
            continue;
        }
        if !skip {
            out.push(chars[i]);
        }
        i += 1;
    }

    // Strip remaining HTML tags.
    let out = strip_html(&out);

    // Decode common HTML entities.
    let out = out
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&#39;", "'");

    // Collapse whitespace.
    let out: String = out
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    // If the result still looks like CSS/JS code, discard it entirely.
    if out.contains('{') && out.contains('}') && out.contains(':') {
        return String::new();
    }

    // Truncate.
    if out.len() > 300 {
        format!("{}…", &out[..300])
    } else {
        out
    }
}

/// Abbreviate a long dictionary title by taking the first letter of each
/// significant word (uppercase or title-case). Examples:
/// - "Merriam-Webster's Advanced Learner's English Dictionary" → "MWALED"
/// - "The American Heritage® Dictionary of the English Language, Fifth Edition © 2017" → "TAHDOTELFE"
/// - "Longman Dictionary of Contemporary English" → "LDOCE"
fn abbreviate_title(title: &str) -> String {
    let skip_words = ["the", "of", "a", "an", "and", "or", "for", "in", "on", "to"];
    let mut abbr = String::new();
    for word in title.split_whitespace() {
        // Strip punctuation/copyright symbols from the word boundary.
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphabetic() || *c == '-' || *c == '\'')
            .collect();
        if clean.is_empty() || skip_words.contains(&clean.to_lowercase().as_str()) {
            continue;
        }
        // Take the first character of each significant word (uppercase it).
        if let Some(c) = clean.chars().next() {
            abbr.push(c.to_ascii_uppercase());
        }
    }
    if abbr.len() > 10 {
        abbr.truncate(10);
    }
    abbr
}

pub struct MdxDictionary {
    stem: String,
    display_name: String,
    short_name: String,
    mdx_path: String,
    mdd_path: Option<String>,
    fst: RwLock<Option<Arc<FstIndex>>>,
    mdd_fst: RwLock<Option<Arc<FstIndex>>>,
    mdx_file: RwLock<Option<Arc<File>>>,
    mdd_file: RwLock<Option<Arc<File>>>,
}

impl MdxDictionary {
    pub fn new(mdx_path: &str) -> Self {
        let stem = PathBuf::from(mdx_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(mdx_path)
            .to_string();

        // Read header to get the display title.
        let display_name = std::fs::read(mdx_path)
            .ok()
            .and_then(|data| {
                let len = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
                let buf = data.get(4..4 + len)?;
                use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
                let xml = UTF_16LE.decode(buf, DecoderTrap::Strict).ok()?;
                let attrs = parser::header::parse_attrs(&xml);
                attrs.get("Title").cloned()
            })
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| stem.clone());

        let auto_short = if display_name.len() <= 25 {
            display_name.clone()
        } else {
            abbreviate_title(&display_name)
        };

        // Use user override from settings if set, otherwise auto-generated.
        let short_name = crate::settings::short_name_override(mdx_path).unwrap_or(auto_short);

        let mdd_path = {
            let p = PathBuf::from(mdx_path).with_extension("mdd");
            if p.exists() {
                Some(p.to_string_lossy().into_owned())
            } else {
                None
            }
        };

        MdxDictionary {
            stem,
            display_name,
            short_name,
            mdx_path: mdx_path.to_string(),
            mdd_path,
            fst: RwLock::new(None),
            mdd_fst: RwLock::new(None),
            mdx_file: RwLock::new(None),
            mdd_file: RwLock::new(None),
        }
    }

    fn open_fst(&self) -> Option<Arc<FstIndex>> {
        {
            if let Some(idx) = self.fst.read().unwrap().as_ref() {
                return Some(idx.clone());
            }
        }
        let fst_path = format!("{}.fst", self.mdx_path);
        let off_path = format!("{}.offsets", self.mdx_path);
        match FstIndex::open(&fst_path, &off_path) {
            Some(idx) => {
                *self.fst.write().unwrap() = Some(idx.clone());
                Some(idx)
            }
            None => {
                warn!("{}: cannot open FST index {fst_path}", self.display_name);
                None
            }
        }
    }

    fn open_mdd_fst(&self) -> Option<Arc<FstIndex>> {
        let mdd_path = self.mdd_path.as_ref()?;
        {
            if let Some(idx) = self.mdd_fst.read().unwrap().as_ref() {
                return Some(idx.clone());
            }
        }
        let fst_path = format!("{mdd_path}.fst");
        let off_path = format!("{mdd_path}.offsets");
        match FstIndex::open(&fst_path, &off_path) {
            Some(idx) => {
                *self.mdd_fst.write().unwrap() = Some(idx.clone());
                Some(idx)
            }
            None => {
                warn!(
                    "{}: cannot open MDD FST index {fst_path}",
                    self.display_name
                );
                None
            }
        }
    }

    fn open_mdx_file(&self) -> Option<Arc<File>> {
        {
            if let Some(f) = self.mdx_file.read().unwrap().as_ref() {
                return Some(f.clone());
            }
        }
        match File::open(&self.mdx_path) {
            Ok(f) => {
                let arc = Arc::new(f);
                *self.mdx_file.write().unwrap() = Some(arc.clone());
                Some(arc)
            }
            Err(e) => {
                warn!("{}: cannot open {}: {e}", self.display_name, self.mdx_path);
                None
            }
        }
    }

    fn open_mdd_file(&self) -> Option<Arc<File>> {
        let mdd_path = self.mdd_path.as_ref()?;
        {
            if let Some(f) = self.mdd_file.read().unwrap().as_ref() {
                return Some(f.clone());
            }
        }
        match File::open(mdd_path) {
            Ok(f) => {
                let arc = Arc::new(f);
                *self.mdd_file.write().unwrap() = Some(arc.clone());
                Some(arc)
            }
            Err(e) => {
                warn!("{}: cannot open {mdd_path}: {e}", self.display_name);
                None
            }
        }
    }

    fn invalidate_cache(&self) {
        *self.fst.write().unwrap() = None;
        *self.mdd_fst.write().unwrap() = None;
        *self.mdx_file.write().unwrap() = None;
        *self.mdd_file.write().unwrap() = None;
    }

    fn read_entry_bytes(&self, rec: &[u8]) -> Option<Vec<u8>> {
        let (file_offset, csize, dsize, start, end) = decode_offset(rec)?;
        let file = self.open_mdx_file()?;
        read_entry(&file, file_offset, csize, dsize, start, end)
    }

    fn read_mdd_entry_bytes(&self, rec: &[u8]) -> Option<Vec<u8>> {
        let (file_offset, csize, dsize, start, end) = decode_offset(rec)?;
        let file = self.open_mdd_file()?;
        read_entry(&file, file_offset, csize, dsize, start, end)
    }
}

impl Dictionary for MdxDictionary {
    fn name(&self) -> &str {
        &self.display_name
    }

    fn short_name(&self) -> &str {
        &self.short_name
    }

    fn stem(&self) -> &str {
        &self.stem
    }

    fn info(&self) -> crate::dictionary::DictInfo {
        use crate::dictionary::DictInfo;
        let (version, encoding, title, description) = std::fs::read(&self.mdx_path)
            .ok()
            .and_then(|data| {
                let len = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
                let buf = data.get(4..4 + len)?;
                use encoding::{DecoderTrap, Encoding, all::UTF_16LE};
                let xml = UTF_16LE.decode(buf, DecoderTrap::Strict).ok()?;
                let attrs = parser::header::parse_attrs(&xml);
                let version = attrs
                    .get("GeneratedByEngineVersion")
                    .and_then(|v| v.trim().chars().next())
                    .and_then(|c| c.to_digit(10))
                    .unwrap_or(2) as u8;
                let encoding = attrs.get("Encoding").cloned().unwrap_or_default();
                let title = attrs.get("Title").cloned().unwrap_or_default();
                let description = attrs.get("Description").cloned().unwrap_or_default();
                Some((version, encoding, title, description))
            })
            .unwrap_or((2, String::new(), self.display_name.clone(), String::new()));

        DictInfo {
            title,
            description,
            stem: self.stem.clone(),
            path: self.mdx_path.clone(),
            encoding,
            version,
        }
    }

    fn lookup(&self, word: &str) -> Option<String> {
        let idx = self.open_fst()?;
        let mut target = word.to_lowercase();
        let mut hops: u8 = 0;

        loop {
            let rec = idx.get_record(&target)?.to_vec();
            let raw = self.read_entry_bytes(&rec)?;
            let text = String::from_utf8_lossy(&raw).into_owned();
            match parse_link(&text) {
                Some(next) if hops < MAX_REDIRECTS => {
                    target = next.to_lowercase();
                    hops += 1;
                }
                Some(_) => return None,
                None => return Some(text),
            }
        }
    }

    fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let Some(idx) = self.open_fst() else {
            return vec![];
        };
        let p = prefix.to_lowercase();

        let mut results: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Prefix search — exact, fast, always first.
        let mut stream = idx.map.search(FstStr::new(&p).starts_with()).into_stream();
        while let Some((key, _)) = stream.next() {
            if let Ok(s) = std::str::from_utf8(key) {
                seen.insert(s.to_string());
                results.push(s.to_string());
            }
            if results.len() >= limit {
                break;
            }
        }

        // Fuzzy supplement: fill remaining slots when query is long enough to
        // have meaningful edit distance (avoids noise on 1-2 char inputs).
        if results.len() < limit && p.len() >= 3 {
            let dist = if p.len() < 6 { 1 } else { 2 };
            if let Ok(lev) = Levenshtein::new(&p, dist) {
                let mut fuzz = idx.map.search(&lev).into_stream();
                while let Some((key, _)) = fuzz.next() {
                    if let Ok(s) = std::str::from_utf8(key)
                        && seen.insert(s.to_string())
                    {
                        results.push(s.to_string());
                    }
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }

        results
    }

    fn resource(&self, path: &str) -> Option<Vec<u8>> {
        let idx = self.open_mdd_fst()?;
        let key = normalize_path(path);
        for cand in candidate_keys(&key) {
            if let Some(rec) = idx.get_record(&cand) {
                let rec = rec.to_vec();
                return self.read_mdd_entry_bytes(&rec);
            }
        }
        None
    }

    fn css_resources(&self) -> Vec<(String, String)> {
        let Some(idx) = self.open_mdd_fst() else {
            return vec![];
        };
        let mut stream = idx.map.search(AlwaysMatch).into_stream();
        let mut results = Vec::new();
        while let Some((key, row)) = stream.next() {
            let Ok(name) = std::str::from_utf8(key) else {
                continue;
            };
            if !name.ends_with(".css") {
                continue;
            }
            let start = row as usize * REC_LEN;
            let Some(rec) = idx.offsets.get(start..start + REC_LEN) else {
                continue;
            };
            let rec = rec.to_vec();
            if let Some(data) = self.read_mdd_entry_bytes(&rec) {
                let body = String::from_utf8_lossy(&data).into_owned();
                results.push((name.to_string(), body));
            }
        }
        results
    }

    fn build_index(&self, force: bool) -> anyhow::Result<()> {
        self.invalidate_cache();
        build_mdx_index(&self.mdx_path, force)?;
        if let Some(mdd_path) = &self.mdd_path {
            build_mdd_index(mdd_path, force)?;
        }
        Ok(())
    }

    fn index_ready(&self) -> bool {
        let mdx_ready = PathBuf::from(format!("{}.fst", self.mdx_path)).exists();
        let mdd_ready = self
            .mdd_path
            .as_ref()
            .map(|p| PathBuf::from(format!("{p}.fst")).exists())
            .unwrap_or(true);
        mdx_ready && mdd_ready
    }
}

// ── index builders ────────────────────────────────────────────────────────────

fn build_mdx_index(file: &str, force: bool) -> anyhow::Result<()> {
    let fst_path = format!("{file}.fst");
    let off_path = format!("{file}.offsets");
    if PathBuf::from(&fst_path).exists() {
        if force {
            fs::remove_file(&fst_path)?;
            fs::remove_file(&off_path).ok();
        } else {
            return Ok(());
        }
    }
    let res = (|| -> anyhow::Result<()> {
        info!("indexing MDX {} -> {}", file, fst_path);
        let f = File::open(file)?;
        let mmap = unsafe { memmap2::Mmap::map(&f)? };
        let mdx = Mdx::parse(&mmap)?;

        let mut entries: Vec<(String, [u8; REC_LEN])> = mdx
            .entries
            .iter()
            .map(|e| {
                (
                    e.text.to_lowercase(),
                    encode_offset(
                        e.file_offset,
                        e.block_csize,
                        e.block_dsize,
                        e.start_in_block,
                        e.end_in_block,
                    ),
                )
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|a, b| a.0 == b.0);

        let mut off_writer = BufWriter::new(File::create(&off_path)?);
        let mut builder = MapBuilder::new(BufWriter::new(File::create(&fst_path)?))?;
        for (row, (key, rec)) in entries.iter().enumerate() {
            builder.insert(key.as_bytes(), row as u64)?;
            off_writer.write_all(rec)?;
        }
        builder.finish()?;
        info!("MDX indexed {} entries: {}", entries.len(), fst_path);
        Ok(())
    })();

    if res.is_err() {
        fs::remove_file(&fst_path).ok();
        fs::remove_file(&off_path).ok();
    }
    res
}

fn build_mdd_index(file: &str, force: bool) -> anyhow::Result<()> {
    let fst_path = format!("{file}.fst");
    let off_path = format!("{file}.offsets");
    let needs = if PathBuf::from(&fst_path).exists() {
        force || mdd_fst_empty(&fst_path)
    } else {
        true
    };
    if !needs {
        return Ok(());
    }
    if PathBuf::from(&fst_path).exists() {
        fs::remove_file(&fst_path)?;
        fs::remove_file(&off_path).ok();
    }
    let res = (|| -> anyhow::Result<()> {
        info!("indexing MDD {} -> {}", file, fst_path);
        let f = File::open(file)?;
        let mmap = unsafe { memmap2::Mmap::map(&f)? };
        let mdd = Mdd::parse(&mmap)?;

        let mut entries: Vec<(String, [u8; REC_LEN])> = mdd
            .entries
            .iter()
            .map(|e| {
                (
                    normalize_path(&e.path),
                    encode_offset(
                        e.file_offset,
                        e.block_csize,
                        e.block_dsize,
                        e.start_in_block,
                        e.end_in_block,
                    ),
                )
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|a, b| a.0 == b.0);

        let mut off_writer = BufWriter::new(File::create(&off_path)?);
        let mut builder = MapBuilder::new(BufWriter::new(File::create(&fst_path)?))?;
        for (row, (key, rec)) in entries.iter().enumerate() {
            builder.insert(key.as_bytes(), row as u64)?;
            off_writer.write_all(rec)?;
        }
        builder.finish()?;
        info!("MDD indexed {} entries: {}", entries.len(), fst_path);
        Ok(())
    })();

    if res.is_err() {
        fs::remove_file(&fst_path).ok();
        fs::remove_file(&off_path).ok();
    }
    res
}

fn mdd_fst_empty(fst_path: &str) -> bool {
    File::open(fst_path)
        .ok()
        .and_then(|f| unsafe { Mmap::map(&f) }.ok())
        .and_then(|m| Map::new(m).ok())
        .map(|m| m.is_empty())
        .unwrap_or(true)
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn parse_link(def: &str) -> Option<String> {
    let rest = def.strip_prefix("@@@LINK=")?;
    let target: String = rest
        .chars()
        .take_while(|c| *c != '\r' && *c != '\n' && *c != '\0')
        .collect();
    let trimmed = target.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn candidate_keys(key: &str) -> Vec<String> {
    let mut v = vec![key.to_string()];
    if !key.contains('/') {
        v.push(format!("sound/{key}"));
        v.push(format!("audio/{key}"));
        v.push(format!("images/{key}"));
        v.push(format!("img/{key}"));
    }
    v.push(format!("/{key}"));
    v
}
