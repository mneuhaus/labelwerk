//! Brother QL raster command language (QL-1100 family).
//!
//! Reference: "Software Developer's Manual, Raster Command Reference QL-1100/1110NWB/1115NWB" v1.00
//! (download.brother.com/welcome/docp100366/cv_ql1100_eng_raster_100.pdf). The command order and header
//! bytes mirror what Brother's own macOS driver (`rastertobrotherQL1100`) emits; `tests/brother_filter.rs`
//! checks that byte for byte.

use anyhow::{Result, bail, ensure};

use crate::bitmap::Bitmap;
use crate::media::{Kind, Media};

/// Pins on the QL-1100 print head.
pub const HEAD_PINS: u32 = 1296;
/// Bytes per raster line (`HEAD_PINS / 8`).
pub const LINE_BYTES: usize = 162;
/// Zero bytes sent first to flush a half-received job ("invalidate").
pub const INVALIDATE_BYTES: usize = 400;
/// Shortest and longest continuous label in raster lines (25.4 mm .. 3 m).
pub const MIN_CONTINUOUS_LINES: u32 = 301;
pub const MAX_CONTINUOUS_LINES: u32 = 35434;
/// Feed margin for continuous tape. Brother documents 3 mm .. 127 mm and P-touch Editor sends 3 mm, while
/// Brother's CUPS driver sends 0; the printer accepts both.
pub const MIN_MARGIN_DOTS: u16 = 35;
pub const MAX_MARGIN_DOTS: u16 = 1500;

pub const ESC: u8 = 0x1B;

/// Status information request (`ESC i S`); the printer answers with 32 bytes.
pub const STATUS_REQUEST: [u8; 3] = [ESC, b'i', b'S'];
/// Initialize / cancel (`ESC @`).
pub const INITIALIZE: [u8; 2] = [ESC, b'@'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintOptions {
    /// Cut after every `cut_every` labels.
    pub auto_cut: bool,
    pub cut_every: u8,
    /// Cut after the last label even when `auto_cut` is off.
    pub cut_at_end: bool,
    /// Blank feed before and after the print on continuous tape, in dots. Ignored for die-cut labels.
    pub margin_dots: u16,
    /// Ask the printer to favour quality over speed.
    pub quality: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self { auto_cut: true, cut_every: 1, cut_at_end: true, margin_dots: MIN_MARGIN_DOTS, quality: false }
    }
}

/// Build the complete byte stream for one job. Every page is one label; repeat a page for copies.
pub fn encode_job(media: &Media, pages: &[&Bitmap], opts: &PrintOptions) -> Result<Vec<u8>> {
    ensure!(!pages.is_empty(), "nothing to print");
    let mut out = Vec::with_capacity(INVALIDATE_BYTES + pages.iter().map(|p| p.height as usize * 8).sum::<usize>());
    out.resize(INVALIDATE_BYTES, 0);
    out.extend_from_slice(&INITIALIZE);

    for (i, page) in pages.iter().enumerate() {
        check_page(media, page)?;
        out.extend_from_slice(&[ESC, b'i', b'a', 0x01]); // raster mode, repeated per page like the driver
        let (w_mm, l_mm) = media.status_size_mm();
        let mut valid = 0x80 | 0x02 | 0x04 | 0x08; // printer recovery + media type, width, length
        if opts.quality {
            valid |= 0x40;
        }
        let lines = page.height.to_le_bytes();
        out.extend_from_slice(&[ESC, b'i', b'z', valid, media.kind.code(), w_mm, l_mm]);
        out.extend_from_slice(&lines);
        out.extend_from_slice(&[(i > 0) as u8, 0]);
        out.extend_from_slice(&[ESC, b'i', b'M', if opts.auto_cut { 0x40 } else { 0 }]);
        out.extend_from_slice(&[ESC, b'i', b'A', opts.cut_every.max(1)]);
        // Bit 3 = cut at end. Bit 1 is undocumented; Brother's driver sets it for all media but the 12 and 24 mm
        // round labels.
        let undocumented = if matches!(media.id, 362 | 363) { 0 } else { 0x02 };
        out.extend_from_slice(&[ESC, b'i', b'K', undocumented | if opts.cut_at_end { 0x08 } else { 0 }]);
        let margin = match media.kind {
            Kind::Continuous => opts.margin_dots.min(MAX_MARGIN_DOTS),
            _ => 0,
        };
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&margin.to_le_bytes());
        out.extend_from_slice(&[b'M', 0x02]); // TIFF (PackBits) compression

        let mut line = [0u8; LINE_BYTES];
        let mut packed = Vec::with_capacity(LINE_BYTES + 2);
        for y in 0..page.height {
            pack_line(media, page.row(y), &mut line);
            packed.clear();
            packbits(&line, &mut packed);
            if packed.len() > LINE_BYTES {
                // Brother: compressed data longer than the line is sent as one literal block instead.
                packed.clear();
                for chunk in line.chunks(128) {
                    packed.push(chunk.len() as u8 - 1);
                    packed.extend_from_slice(chunk);
                }
            }
            out.extend_from_slice(&[b'g', 0x00, packed.len() as u8]);
            out.extend_from_slice(&packed);
        }
        out.push(if i + 1 == pages.len() { 0x1A } else { 0x0C });
    }
    Ok(out)
}

fn check_page(media: &Media, page: &Bitmap) -> Result<()> {
    ensure!(
        page.width == media.print_width,
        "page is {} dots wide, {} needs {}",
        page.width,
        media.label(),
        media.print_width
    );
    match media.kind {
        Kind::Continuous => ensure!(
            (MIN_CONTINUOUS_LINES..=MAX_CONTINUOUS_LINES).contains(&page.height),
            "continuous labels must be {MIN_CONTINUOUS_LINES}..{MAX_CONTINUOUS_LINES} lines, got {}",
            page.height
        ),
        _ => ensure!(
            page.height == media.print_length,
            "{} labels have {} lines, got {}",
            media.label(),
            media.print_length,
            page.height
        ),
    }
    Ok(())
}

/// Head-line bit of page column `x`. The first byte of a line drives the highest pins, and page column 0
/// sits on the lowest printable pin (`pins_left`), as Brother's driver does it.
#[inline]
fn bit_of(media: &Media, x: u32) -> usize {
    (HEAD_PINS - 1 - (media.pins_left + x)) as usize
}

/// Place one row of the page into a full head line.
fn pack_line(media: &Media, row: &[u8], line: &mut [u8; LINE_BYTES]) {
    line.fill(0);
    for (x, &px) in row.iter().enumerate() {
        if px != 0 {
            let bit = bit_of(media, x as u32);
            line[bit / 8] |= 0x80 >> (bit % 8);
        }
    }
}

/// TIFF PackBits: runs of 2+ equal bytes become `(1 - n, byte)`, everything else `(n - 1, bytes...)`.
pub fn packbits(data: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < data.len() {
        let mut run = 1;
        while i + run < data.len() && run < 128 && data[i + run] == data[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1i16 - run as i16) as i8 as u8);
            out.push(data[i]);
            i += run;
            continue;
        }
        let start = i;
        while i < data.len() && i - start < 128 {
            if i + 1 < data.len() && data[i + 1] == data[i] {
                break;
            }
            i += 1;
        }
        if i == start {
            i += 1; // a lone byte right before a run; keep it a literal of one
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&data[start..i]);
    }
}

pub fn unpackbits(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(LINE_BYTES);
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as i8;
        i += 1;
        if n >= 0 {
            let len = n as usize + 1;
            ensure!(i + len <= data.len(), "truncated literal run");
            out.extend_from_slice(&data[i..i + len]);
            i += len;
        } else if n != -128 {
            ensure!(i < data.len(), "truncated repeat run");
            out.extend(std::iter::repeat_n(data[i], (1 - n as isize) as usize));
            i += 1;
        }
    }
    Ok(out)
}

/// A decoded job, for tests and the CLI's `decode` command.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DecodedPage {
    /// Raw `ESC i …` / `M` commands of this page in order, without the raster data.
    pub commands: Vec<Vec<u8>>,
    /// Uncompressed head lines (162 bytes each).
    pub lines: Vec<Vec<u8>>,
    /// 0x0C (more pages follow) or 0x1A (last page).
    pub terminator: u8,
}

pub fn decode_job(data: &[u8]) -> Result<Vec<DecodedPage>> {
    let mut i = 0;
    while i < data.len() && data[i] == 0 {
        i += 1;
    }
    let mut pages = Vec::new();
    let mut page = DecodedPage::default();
    let mut compressed = false;
    while i < data.len() {
        match data[i] {
            ESC => {
                let len = match data.get(i + 1..i + 3) {
                    Some([b'@', _]) | Some([b'@']) => 2,
                    Some([b'i', b'z']) => 13,
                    Some([b'i', b'd']) => 5,
                    Some([b'i', b'S']) => 3,
                    Some([b'i', _]) => 4,
                    _ if data.get(i + 1) == Some(&b'@') => 2,
                    _ => bail!("unknown escape at {i}: {:02x?}", &data[i..(i + 4).min(data.len())]),
                };
                ensure!(i + len <= data.len(), "truncated command at {i}");
                page.commands.push(data[i..i + len].to_vec());
                i += len;
            }
            b'M' => {
                compressed = data.get(i + 1) == Some(&0x02);
                page.commands.push(data[i..i + 2].to_vec());
                i += 2;
            }
            b'g' => {
                ensure!(i + 3 <= data.len(), "truncated raster line at {i}");
                let n = data[i + 2] as usize;
                let payload = data.get(i + 3..i + 3 + n).ok_or_else(|| anyhow::anyhow!("truncated raster data"))?;
                let mut line = if compressed { unpackbits(payload)? } else { payload.to_vec() };
                line.resize(LINE_BYTES, 0);
                page.lines.push(line);
                i += 3 + n;
            }
            b'Z' => {
                page.lines.push(vec![0; LINE_BYTES]);
                i += 1;
            }
            t @ (0x0C | 0x1A) => {
                page.terminator = t;
                pages.push(std::mem::take(&mut page));
                i += 1;
            }
            b => bail!("unexpected byte {b:#04x} at {i}"),
        }
    }
    Ok(pages)
}

/// Turn decoded head lines back into a page bitmap of the given media (inverse of `pack_line`).
pub fn lines_to_bitmap(media: &Media, lines: &[Vec<u8>]) -> Bitmap {
    let mut b = Bitmap::new(media.print_width, lines.len() as u32);
    for (y, line) in lines.iter().enumerate() {
        for x in 0..media.print_width {
            let bit = bit_of(media, x);
            if line[bit / 8] & (0x80 >> (bit % 8)) != 0 {
                b.set(x, y as u32, true);
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_matches_brother_examples() {
        // Raster Command Reference, "Select compression mode" example (prefix).
        let mut data = vec![0u8; 20];
        data.extend_from_slice(&[0x22, 0x22, 0x23, 0xBA, 0xBF, 0xA2, 0x22, 0x2B]);
        let mut out = Vec::new();
        packbits(&data, &mut out);
        assert_eq!(out, [0xED, 0x00, 0xFF, 0x22, 0x05, 0x23, 0xBA, 0xBF, 0xA2, 0x22, 0x2B]);
        // Brother's driver encodes an empty 162-byte line as 81 00 df 00.
        out.clear();
        packbits(&[0; LINE_BYTES], &mut out);
        assert_eq!(out, [0x81, 0x00, 0xDF, 0x00]);
    }

    #[test]
    fn packbits_round_trips() {
        let mut seed = 0x1234_5678u32;
        for _ in 0..500 {
            let mut line = [0u8; LINE_BYTES];
            for b in &mut line {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *b = if seed >> 29 == 0 { (seed >> 16) as u8 } else { (seed >> 31) as u8 * 0xFF };
            }
            let mut out = Vec::new();
            packbits(&line, &mut out);
            assert_eq!(unpackbits(&out).unwrap(), line);
        }
    }

    #[test]
    fn encode_then_decode_restores_pages() {
        let media = Media::by_key("62x29").unwrap();
        let mut page = Bitmap::new(media.print_width, media.print_length);
        for y in 10..40 {
            for x in 5..200 {
                page.set(x, y, (x + y) % 3 == 0);
            }
        }
        let job = encode_job(media, &[&page, &page], &PrintOptions::default()).unwrap();
        let pages = decode_job(&job).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].terminator, 0x0C);
        assert_eq!(pages[1].terminator, 0x1A);
        assert_eq!(lines_to_bitmap(media, &pages[1].lines), page);
        // die-cut: no feed margin
        assert!(pages[0].commands.contains(&vec![ESC, b'i', b'd', 0, 0]));
    }

    #[test]
    fn incompressible_lines_fall_back_to_literals() {
        let media = Media::by_key("103").unwrap();
        let mut page = Bitmap::new(media.print_width, MIN_CONTINUOUS_LINES);
        for x in 0..media.print_width {
            page.set(x, 0, x % 24 < 8 && x % 24 != 0); // "xx 00 00": PackBits would grow to ~200 bytes
        }
        let job = encode_job(media, &[&page], &PrintOptions::default()).unwrap();
        let first_g = job.iter().position(|&b| b == b'g').unwrap();
        assert!(job[first_g + 2] as usize <= LINE_BYTES + 2);
        let pages = decode_job(&job).unwrap();
        assert_eq!(lines_to_bitmap(media, &pages[0].lines), page);
    }

    #[test]
    fn rejects_wrong_page_sizes() {
        let media = Media::by_key("62").unwrap();
        assert!(encode_job(media, &[&Bitmap::new(600, 400)], &PrintOptions::default()).is_err());
        assert!(encode_job(media, &[&Bitmap::new(696, 100)], &PrintOptions::default()).is_err());
    }
}
