use std::io::Read;

use adler32::adler32;
use anyhow::{Context, Result, bail, ensure};
use encoding::label::encoding_from_whatwg_label;
use flate2::read::ZlibDecoder;
use ripemd::{Digest, Ripemd128};

use super::header::Header;
use crate::util::fast_decrypt;

// ── simple sequential byte reader ────────────────────────────────────────────

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        self.data.get(self.pos..self.pos + n).inspect(|_s| {
            self.pos += n;
        })
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }
    fn be_u16(&mut self) -> Option<u16> {
        self.bytes(2)
            .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
    }
    fn be_u32(&mut self) -> Option<u32> {
        self.bytes(4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    }
    fn be_u64(&mut self) -> Option<u64> {
        self.bytes(8)
            .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
    }
    fn skip(&mut self, n: usize) -> bool {
        if self.pos + n > self.data.len() {
            return false;
        }
        self.pos += n;
        true
    }
    fn remaining(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

// ── public types ─────────────────────────────────────────────────────────────

pub struct KeyBlockHeader {
    pub block_num: usize,
    pub key_block_info_len: usize,
    pub key_blocks_len: usize,
}

pub struct KeyBlockSize {
    pub csize: usize,
    pub dsize: usize,
}

/// A key entry: the word text and its byte offset in the decompressed record buffer.
pub struct KeyEntry {
    pub text: String,
    pub record_offset: usize,
}

// ── key block header ─────────────────────────────────────────────────────────

pub fn parse_key_block_header<'a>(
    data: &'a [u8],
    header: &Header,
) -> Result<(&'a [u8], KeyBlockHeader)> {
    let mut r = Reader::new(data);
    let kbh = if header.version >= 2 {
        let buf = r.bytes(40).context("mdict: truncated key block header")?;
        let checksum = r
            .be_u32()
            .context("mdict: truncated key block header checksum")?;
        ensure!(
            adler32(buf).context("mdict: failed to compute key block header checksum")? == checksum,
            "mdict: key block header checksum mismatch"
        );
        let mut br = Reader::new(buf);
        let block_num =
            br.be_u64()
                .context("mdict: truncated key block header block_num")? as usize;
        let _entry_num = br
            .be_u64()
            .context("mdict: truncated key block header entry_num")?;
        let _info_dsize = br
            .be_u64()
            .context("mdict: truncated key block header info_dsize")?;
        let info_len = br
            .be_u64()
            .context("mdict: truncated key block header info_len")? as usize;
        let blocks_len =
            br.be_u64()
                .context("mdict: truncated key block header blocks_len")? as usize;
        KeyBlockHeader {
            block_num,
            key_block_info_len: info_len,
            key_blocks_len: blocks_len,
        }
    } else {
        let buf = r.bytes(16).context("mdict: truncated key block header")?;
        let mut br = Reader::new(buf);
        let block_num =
            br.be_u32()
                .context("mdict: truncated key block header block_num")? as usize;
        let _entry_num = br
            .be_u32()
            .context("mdict: truncated key block header entry_num")?;
        let info_len = br
            .be_u32()
            .context("mdict: truncated key block header info_len")? as usize;
        let blocks_len =
            br.be_u32()
                .context("mdict: truncated key block header blocks_len")? as usize;
        KeyBlockHeader {
            block_num,
            key_block_info_len: info_len,
            key_blocks_len: blocks_len,
        }
    };
    Ok((r.remaining(), kbh))
}

// ── key block info (compressed sizes) ────────────────────────────────────────

pub fn parse_key_block_info<'a>(
    data: &'a [u8],
    info_len: usize,
    header: &Header,
    block_num: usize,
) -> Result<(&'a [u8], Vec<KeyBlockSize>)> {
    ensure!(
        info_len <= data.len(),
        "mdict: key block info length exceeds remaining input ({} > {})",
        info_len,
        data.len()
    );
    let buf = &data[..info_len];
    let rest = &data[info_len..];

    let sizes = if header.version >= 2 {
        // First 4 bytes must be 0x02000000; may be encrypted+zlib-compressed.
        ensure!(buf.len() >= 8, "mdict: truncated v2 key block info header");
        ensure!(
            &buf[0..4] == b"\x02\x00\x00\x00",
            "mdict: key block info magic mismatch"
        );
        let mut decompressed = Vec::new();
        if header.encrypted & 0x02 != 0 {
            // Encrypt key: ripemd128(checksum_bytes ++ 0x3695_le32)
            let mut md = Ripemd128::new();
            let mut seed = buf[4..8].to_vec();
            seed.extend_from_slice(&0x3695u32.to_le_bytes());
            md.update(&seed);
            let key = md.finalize();
            let decrypted = fast_decrypt(&buf[8..], &key);
            ZlibDecoder::new(&decrypted[..])
                .read_to_end(&mut decompressed)
                .context("mdict: failed to decompress encrypted v2 key block info")?;
        } else {
            ZlibDecoder::new(&buf[8..])
                .read_to_end(&mut decompressed)
                .context("mdict: failed to decompress v2 key block info")?;
        }
        decode_info_v2(&decompressed, &header.encoding, block_num)?
    } else {
        decode_info_v1(buf, &header.encoding, block_num)?
    };

    Ok((rest, sizes))
}

/// V1 key block info entry: be_u32 entries | u8 first_char_count | first_chars | u8 last_char_count | last_chars | be_u32 csize | be_u32 dsize
fn decode_info_v1(data: &[u8], encoding: &str, block_num: usize) -> Result<Vec<KeyBlockSize>> {
    let (mult, term) = char_widths(encoding);
    let mut r = Reader::new(data);
    let mut out = Vec::with_capacity(block_num);
    for _ in 0..block_num {
        let Some(_entries) = r.be_u32() else {
            bail!("mdict: truncated v1 key block info")
        };
        let Some(first_chars) = r.u8() else {
            bail!("mdict: truncated v1 key block info")
        };
        ensure!(
            r.skip(first_chars as usize * mult + term),
            "mdict: truncated v1 key block info (first_chars)"
        );
        let Some(last_chars) = r.u8() else {
            bail!("mdict: truncated v1 key block info")
        };
        ensure!(
            r.skip(last_chars as usize * mult + term),
            "mdict: truncated v1 key block info (last_chars)"
        );
        let Some(csize) = r.be_u32() else {
            bail!("mdict: truncated v1 key block info")
        };
        let Some(dsize) = r.be_u32() else {
            bail!("mdict: truncated v1 key block info")
        };
        out.push(KeyBlockSize {
            csize: csize as usize,
            dsize: dsize as usize,
        });
    }
    Ok(out)
}

/// V2 key block info entry: be_u64 entries | be_u16 first_char_count | first_chars | be_u16 last_char_count | last_chars | be_u64 csize | be_u64 dsize
fn decode_info_v2(data: &[u8], encoding: &str, block_num: usize) -> Result<Vec<KeyBlockSize>> {
    let (mult, term) = char_widths(encoding);
    let mut r = Reader::new(data);
    let mut out = Vec::with_capacity(block_num);
    for _ in 0..block_num {
        let Some(_entries) = r.be_u64() else {
            bail!("mdict: truncated v2 key block info")
        };
        let Some(first_chars) = r.be_u16() else {
            bail!("mdict: truncated v2 key block info")
        };
        ensure!(
            r.skip(first_chars as usize * mult + term),
            "mdict: truncated v2 key block info (first_chars)"
        );
        let Some(last_chars) = r.be_u16() else {
            bail!("mdict: truncated v2 key block info")
        };
        ensure!(
            r.skip(last_chars as usize * mult + term),
            "mdict: truncated v2 key block info (last_chars)"
        );
        let Some(csize) = r.be_u64() else {
            bail!("mdict: truncated v2 key block info")
        };
        let Some(dsize) = r.be_u64() else {
            bail!("mdict: truncated v2 key block info")
        };
        out.push(KeyBlockSize {
            csize: csize as usize,
            dsize: dsize as usize,
        });
    }
    Ok(out)
}

// ── key blocks ───────────────────────────────────────────────────────────────

pub fn parse_key_blocks<'a>(
    data: &'a [u8],
    blocks_len: usize,
    header: &Header,
    sizes: &[KeyBlockSize],
) -> Result<(&'a [u8], Vec<KeyEntry>)> {
    ensure!(
        blocks_len <= data.len(),
        "mdict: key blocks length exceeds remaining input ({} > {})",
        blocks_len,
        data.len()
    );
    let mut buf = &data[..blocks_len];
    let rest = &data[blocks_len..];
    let offset_size = if header.version >= 2 { 8 } else { 4 };

    let mut entries = Vec::new();
    for size in sizes {
        ensure!(
            size.csize <= buf.len(),
            "mdict: key block compressed size exceeds remaining key block buffer ({} > {})",
            size.csize,
            buf.len()
        );
        let decompressed = decompress_block(buf, size.csize, size.dsize)
            .context("mdict: failed to decompress key block")?;
        buf = &buf[size.csize..];
        decode_key_entries(&decompressed, &header.encoding, offset_size, &mut entries)
            .context("mdict: failed to decode key entries")?;
    }

    Ok((rest, entries))
}

/// Decompress one key block (encrypt type in low nibble of first le_u32).
fn decompress_block(buf: &[u8], csize: usize, dsize: usize) -> Result<Vec<u8>> {
    ensure!(csize >= 8, "mdict: key block csize too small ({csize})");
    ensure!(
        buf.len() >= csize,
        "mdict: key block buffer shorter than csize ({} < {})",
        buf.len(),
        csize
    );
    let enc = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let enc_method = (enc >> 4) & 0xf;
    let comp_method = enc & 0xf;
    let checksum = &buf[4..8];
    let payload = &buf[8..csize];

    let mut md = Ripemd128::new();
    md.update(checksum);
    let key = md.finalize();

    let plain: Vec<u8> = match enc_method {
        0 => payload.to_vec(),
        1 => fast_decrypt(payload, &key),
        _ => payload.to_vec(),
    };

    match comp_method {
        0 => Ok(plain),
        1 => {
            let mut out = vec![0u8; dsize];
            let (_, err) = rust_lzo::LZOContext::decompress_to_slice(&plain, &mut out);
            ensure!(
                err == rust_lzo::LZOError::OK,
                "mdict: LZO key block decompression failed"
            );
            Ok(out)
        }
        2 => {
            let mut v = Vec::new();
            ZlibDecoder::new(&plain[..])
                .read_to_end(&mut v)
                .context("mdict: zlib key block decompression failed")?;
            Ok(v)
        }
        _ => bail!("mdict: unknown key block compression method {comp_method}"),
    }
}

/// Walk one decompressed key block and collect (offset, text) pairs.
fn decode_key_entries(
    data: &[u8],
    encoding: &str,
    offset_size: usize,
    out: &mut Vec<KeyEntry>,
) -> Result<()> {
    let utf16 = is_utf16(encoding);
    let term = if utf16 { 2 } else { 1 };
    let mut pos = 0;

    while pos < data.len() {
        ensure!(
            pos + offset_size <= data.len(),
            "mdict: truncated key entry offset"
        );
        let offset = if offset_size == 8 {
            let v = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
            pos += 8;
            v
        } else {
            let v = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            v
        };

        // Find null terminator
        let text_len = if utf16 {
            let mut l = 0;
            while pos + l + 1 < data.len() {
                if data[pos + l] == 0 && data[pos + l + 1] == 0 {
                    break;
                }
                l += 2;
            }
            l
        } else {
            let mut l = 0;
            while pos + l < data.len() && data[pos + l] != 0 {
                l += 1;
            }
            l
        };

        ensure!(
            pos + text_len + term <= data.len(),
            "mdict: truncated key entry text"
        );
        let text = decode_text(&data[pos..pos + text_len], encoding);
        out.push(KeyEntry {
            text,
            record_offset: offset,
        });
        pos += text_len + term;
    }
    Ok(())
}

fn decode_text(bytes: &[u8], encoding: &str) -> String {
    if is_utf16(encoding) {
        let words: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&words).to_string()
    } else if encoding.is_empty() || encoding.eq_ignore_ascii_case("utf-8") {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        encoding_from_whatwg_label(encoding)
            .and_then(|enc| enc.decode(bytes, encoding::DecoderTrap::Ignore).ok())
            .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
    }
}

fn char_widths(encoding: &str) -> (usize, usize) {
    if is_utf16(encoding) { (2, 2) } else { (1, 1) }
}

fn is_utf16(encoding: &str) -> bool {
    let lower = encoding.to_ascii_lowercase();
    lower.contains("utf-16") || lower.contains("utf16")
}
