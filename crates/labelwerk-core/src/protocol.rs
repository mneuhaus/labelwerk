//! Brother raster command language, as spoken by the QL and PT (P-touch) series.
//!
//! References: "Raster Command Reference QL-1100/1110NWB/1115NWB" v1.00 and "Raster Command Reference
//! PT-E550W/P750W/P710BT" v1.02 (download.brother.com). For the QL-1100 the command order and header bytes
//! mirror what Brother's own macOS driver (`rastertobrotherQL1100`) emits; `tests/brother_filter.rs` checks
//! that byte for byte.

use anyhow::{Result, bail, ensure};

use crate::bitmap::Bitmap;
use crate::media::{Kind, Media};
use crate::model::{Family, ModeCommand, Model, RasterCommand};

/// Feed margin limit on continuous tape (127 mm at 300 dpi; PT models document 127 mm as well).
pub const MAX_MARGIN_MM: f32 = 127.0;

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
    /// Cut (QL) or feed and cut (PT, "no chain printing") after the last label.
    pub cut_at_end: bool,
    /// Blank feed before and after the print on continuous tape, in dots; `None` = the model's default.
    pub margin_dots: Option<u16>,
    /// Ask the printer to favour quality over speed.
    pub quality: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self { auto_cut: true, cut_every: 1, cut_at_end: true, margin_dots: None, quality: false }
    }
}

/// Feed margin actually sent for a medium.
pub fn margin_dots(model: &Model, media: &Media, opts: &PrintOptions) -> u16 {
    match media.kind {
        Kind::Continuous => {
            let max = crate::media::mm_to_dots(MAX_MARGIN_MM, model.dpi) as u16;
            opts.margin_dots.unwrap_or(model.default_margin_dots).min(max)
        }
        _ => 0,
    }
}

/// Build the complete byte stream for one job. Every page is one label; repeat a page for copies.
pub fn encode_job(model: &Model, media: &Media, pages: &[&Bitmap], opts: &PrintOptions) -> Result<Vec<u8>> {
    ensure!(!pages.is_empty(), "nothing to print");
    let proto = &model.protocol;
    let line_bytes = model.line_bytes();
    let mut out = Vec::with_capacity(proto.invalidate + pages.iter().map(|p| p.height as usize * 8).sum::<usize>());
    out.resize(proto.invalidate, 0);
    out.extend_from_slice(&INITIALIZE);

    for (i, page) in pages.iter().enumerate() {
        check_page(model, media, page)?;
        match proto.mode {
            // repeated per page, like Brother's drivers do
            ModeCommand::DynamicMode => out.extend_from_slice(&[ESC, b'i', b'a', 0x01]),
            ModeCommand::GraphicsMode => out.extend_from_slice(&[ESC, b'i', b'R', 0x01]),
            ModeCommand::None => {}
        }
        if proto.info {
            let (w_mm, l_mm) = media.status_size_mm();
            let mut valid = proto.info_flags;
            if opts.quality {
                valid |= 0x40;
            }
            let last = i + 1 == pages.len();
            let position = match (proto.last_page_flag && last, i) {
                (true, _) => 2,
                (false, 0) => 0,
                (false, _) => 1,
            };
            out.extend_from_slice(&[ESC, b'i', b'z', valid, media.media_type, w_mm, l_mm]);
            out.extend_from_slice(&page.height.to_le_bytes());
            out.extend_from_slice(&[position, 0]);
        }
        if proto.cutter {
            out.extend_from_slice(&[ESC, b'i', b'M', if opts.auto_cut { 0x40 } else { 0 }]);
        }
        if proto.cut_every {
            out.extend_from_slice(&[ESC, b'i', b'A', opts.cut_every.max(1)]);
        }
        if proto.expanded {
            let end = if opts.cut_at_end { 0x08 } else { 0 };
            let expanded = match model.family {
                // Bit 1 is undocumented; Brother's QL-1100 driver sets it for all media but the 12 and 24 mm round
                // labels.
                Family::Ql if model.head_pins == 1296 && !matches!(media.id, 362 | 363) => end | 0x02,
                _ => end,
            };
            out.extend_from_slice(&[ESC, b'i', b'K', expanded]);
        }
        if proto.d460bt_magic {
            // ptouch-print: margin 1, then "4D 00" (no compression), or the print comes out corrupted
            out.extend_from_slice(&[ESC, b'i', b'd', 0x01, 0x00, b'M', 0x00]);
        } else {
            out.extend_from_slice(&[ESC, b'i', b'd']);
            out.extend_from_slice(&margin_dots(model, media, opts).to_le_bytes());
            if proto.compression {
                out.extend_from_slice(&[b'M', 0x02]); // TIFF (PackBits)
            }
        }

        let mut line = vec![0u8; line_bytes];
        let mut packed = Vec::with_capacity(line_bytes + 2);
        for y in 0..page.height {
            pack_line(model, media, page.row(y), &mut line);
            let data: &[u8] = if proto.compression && !proto.d460bt_magic {
                packed.clear();
                packbits(&line, &mut packed);
                if packed.len() > line_bytes {
                    // Brother: compressed data longer than the line is sent as literal blocks instead.
                    packed.clear();
                    for chunk in line.chunks(128) {
                        packed.push(chunk.len() as u8 - 1);
                        packed.extend_from_slice(chunk);
                    }
                }
                &packed
            } else {
                &line
            };
            match proto.raster {
                RasterCommand::Lower => out.extend_from_slice(&[b'g', 0x00, data.len() as u8]),
                RasterCommand::Upper => {
                    out.push(b'G');
                    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
                }
            }
            out.extend_from_slice(data);
        }
        out.push(if i + 1 == pages.len() { 0x1A } else { 0x0C });
    }
    Ok(out)
}

fn check_page(model: &Model, media: &Media, page: &Bitmap) -> Result<()> {
    ensure!(
        page.width == media.print_width,
        "page is {} dots wide, {} needs {}",
        page.width,
        media.label(),
        media.print_width
    );
    match media.kind {
        Kind::Continuous => {
            let (min, max) = model.length_limits();
            ensure!((min..=max).contains(&page.height), "continuous labels must be {min}..{max} lines, got {}", page.height)
        }
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
fn bit_of(model: &Model, media: &Media, x: u32) -> usize {
    (model.head_pins - 1 - (media.pins_left + x)) as usize
}

/// Place one row of the page into a full head line.
fn pack_line(model: &Model, media: &Media, row: &[u8], line: &mut [u8]) {
    line.fill(0);
    for (x, &px) in row.iter().enumerate() {
        if px != 0 {
            let bit = bit_of(model, media, x as u32);
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
    let mut out = Vec::with_capacity(data.len() * 2);
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
    /// Uncompressed head lines.
    pub lines: Vec<Vec<u8>>,
    /// 0x0C (more pages follow) or 0x1A (last page).
    pub terminator: u8,
}

pub fn decode_job(model: &Model, data: &[u8]) -> Result<Vec<DecodedPage>> {
    let line_bytes = model.line_bytes();
    let mut i = 0;
    while i < data.len() && data[i] == 0 {
        i += 1;
    }
    let mut pages = Vec::new();
    let mut page = DecodedPage::default();
    let mut compressed = false;
    let push_line = |page: &mut DecodedPage, payload: &[u8], compressed: bool| -> Result<()> {
        let mut line = if compressed { unpackbits(payload)? } else { payload.to_vec() };
        line.resize(line_bytes, 0);
        page.lines.push(line);
        Ok(())
    };
    while i < data.len() {
        match data[i] {
            ESC => {
                let len = match data.get(i + 1..i + 3) {
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
                push_line(&mut page, payload, compressed)?;
                i += 3 + n;
            }
            b'G' => {
                ensure!(i + 3 <= data.len(), "truncated raster line at {i}");
                let n = u16::from_le_bytes([data[i + 1], data[i + 2]]) as usize;
                let payload = data.get(i + 3..i + 3 + n).ok_or_else(|| anyhow::anyhow!("truncated raster data"))?;
                push_line(&mut page, payload, compressed)?;
                i += 3 + n;
            }
            b'Z' => {
                page.lines.push(vec![0; line_bytes]);
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
pub fn lines_to_bitmap(model: &Model, media: &Media, lines: &[Vec<u8>]) -> Bitmap {
    let mut b = Bitmap::new(media.print_width, lines.len() as u32);
    for (y, line) in lines.iter().enumerate() {
        for x in 0..media.print_width {
            let bit = bit_of(model, media, x);
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

    fn ql1100() -> &'static Model {
        Model::by_name("QL-1100").unwrap()
    }

    fn p710bt() -> &'static Model {
        Model::by_name("PT-P710BT").unwrap()
    }

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
        packbits(&[0; 162], &mut out);
        assert_eq!(out, [0x81, 0x00, 0xDF, 0x00]);
    }

    #[test]
    fn packbits_round_trips() {
        let mut seed = 0x1234_5678u32;
        for _ in 0..500 {
            let mut line = [0u8; 162];
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
        let model = ql1100();
        let media = model.media_by_key("62x29").unwrap();
        let mut page = Bitmap::new(media.print_width, media.print_length);
        for y in 10..40 {
            for x in 5..200 {
                page.set(x, y, (x + y) % 3 == 0);
            }
        }
        let job = encode_job(model, media, &[&page, &page], &PrintOptions::default()).unwrap();
        let pages = decode_job(model, &job).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].terminator, 0x0C);
        assert_eq!(pages[1].terminator, 0x1A);
        assert_eq!(lines_to_bitmap(model, media, &pages[1].lines), page);
        // die-cut: no feed margin
        assert!(pages[0].commands.contains(&vec![ESC, b'i', b'd', 0, 0]));
    }

    #[test]
    fn incompressible_lines_fall_back_to_literals() {
        let model = ql1100();
        let media = model.media_by_key("103").unwrap();
        let mut page = Bitmap::new(media.print_width, model.length_limits().0);
        for x in 0..media.print_width {
            page.set(x, 0, x % 24 < 8 && x % 24 != 0); // "xx 00 00": PackBits would grow to ~200 bytes
        }
        let job = encode_job(model, media, &[&page], &PrintOptions::default()).unwrap();
        let first_g = job.iter().position(|&b| b == b'g').unwrap();
        assert!(job[first_g + 2] as usize <= 162 + 2);
        let pages = decode_job(model, &job).unwrap();
        assert_eq!(lines_to_bitmap(model, media, &pages[0].lines), page);
    }

    #[test]
    fn rejects_wrong_page_sizes() {
        let model = ql1100();
        let media = model.media_by_key("62").unwrap();
        assert!(encode_job(model, media, &[&Bitmap::new(600, 400)], &PrintOptions::default()).is_err());
        assert!(encode_job(model, media, &[&Bitmap::new(696, 100)], &PrintOptions::default()).is_err());
    }

    fn job_for(name: &str, key: &str, lines: u32, pages: usize) -> (Vec<u8>, Vec<DecodedPage>) {
        let model = Model::by_name(name).unwrap();
        let media = model.media_by_key(key).unwrap();
        let lines = if media.kind == Kind::Continuous { lines } else { media.print_length };
        let mut page = Bitmap::new(media.print_width, lines);
        page.set(0, 0, true);
        let refs: Vec<&Bitmap> = std::iter::repeat_n(&page, pages).collect();
        let job = encode_job(model, media, &refs, &PrintOptions::default()).unwrap();
        let decoded = decode_job(model, &job).unwrap();
        assert_eq!(lines_to_bitmap(model, media, &decoded[0].lines), page, "{name} round trip");
        (job, decoded)
    }

    /// QL-800 Raster Command Reference: no compression, 400 invalidate bytes, raw 90-byte `g` lines.
    #[test]
    fn ql800_sends_uncompressed_lines() {
        let (job, pages) = job_for("QL-800", "62", 400, 1);
        assert!(job[..400].iter().all(|&b| b == 0) && job[400..402] == INITIALIZE);
        assert!(!pages[0].commands.iter().any(|c| c[0] == b'M'));
        let first_g = job.iter().position(|&b| b == b'g').unwrap();
        assert_eq!(job[first_g + 2], 90);
    }

    /// QL-500: raster mode only, no mode switch, no cutter, no expanded mode.
    #[test]
    fn ql500_sends_only_what_it_knows() {
        let (_, pages) = job_for("QL-500", "62", 400, 1);
        let starts: Vec<[u8; 3]> = pages[0].commands.iter().map(|c| [c[0], c[1], *c.get(2).unwrap_or(&0)]).collect();
        assert!(!starts.contains(&[ESC, b'i', b'a']));
        assert!(!starts.contains(&[ESC, b'i', b'M']));
        assert!(!starts.contains(&[ESC, b'i', b'K']));
    }

    /// PT-P900 family: n9 = 2 marks the last page.
    #[test]
    fn p900_marks_the_last_page() {
        let (_, pages) = job_for("PT-P900W", "24", 200, 2);
        let info = |p: &DecodedPage| p.commands.iter().find(|c| c.starts_with(&[ESC, b'i', b'z'])).unwrap().clone();
        assert_eq!(info(&pages[0])[11], 0);
        assert_eq!(info(&pages[1])[11], 2);
    }

    /// PT-D460BT group (ptouch-print): margin magic with "4D 00", uncompressed G lines, n9 = 2.
    #[test]
    fn d460bt_magic_sequence() {
        let (job, pages) = job_for("PT-D460BT", "18", 200, 1); // 18 mm is its widest tape
        assert!(job.windows(7).any(|w| w == [ESC, b'i', b'd', 0x01, 0x00, b'M', 0x00]));
        assert!(!pages[0].commands.iter().any(|c| c.starts_with(&[ESC, b'i', b'K'])));
        assert!(job.windows(3).any(|w| w == [b'G', 16, 0]));
    }

    /// Raster Command Reference PT-E550W/P750W/P710BT, 2.1: "printing 100 mm on 24-mm-wide tape with the
    /// 180 dpi model" -> ESC i z with 0x2AA (682) lines, and 2 mm margins = 14 dots.
    #[test]
    fn p710bt_job_follows_the_reference() {
        let model = p710bt();
        let media = model.media_by_key("24").unwrap();
        let mut page = Bitmap::new(media.print_width, 682);
        page.set(0, 0, true);
        page.set(127, 1, true);
        let job = encode_job(model, media, &[&page], &PrintOptions::default()).unwrap();
        assert!(job[..100].iter().all(|&b| b == 0) && job[100..102] == INITIALIZE);
        let pages = decode_job(model, &job).unwrap();
        let cmds = &pages[0].commands;
        assert_eq!(cmds[1], [ESC, b'i', b'a', 0x01]);
        assert_eq!(cmds[2], [ESC, b'i', b'z', 0x84, 0x01, 0x18, 0x00, 0xAA, 0x02, 0, 0, 0, 0]);
        assert_eq!(cmds[3], [ESC, b'i', b'M', 0x40]);
        assert_eq!(cmds[4], [ESC, b'i', b'K', 0x08], "no ESC i A on the P710BT, no chain printing");
        assert_eq!(cmds[5], [ESC, b'i', b'd', 14, 0]);
        assert_eq!(cmds[6], [b'M', 0x02]);
        assert!(job.windows(3).any(|w| w[0] == b'G' && w[2] == 0), "uppercase G with 16-bit length");
        assert_eq!(pages[0].lines[0].len(), 16);
        assert_eq!(lines_to_bitmap(model, media, &pages[0].lines), page);
        // 24 mm uses every pin: column 0 is pin 0 (last bit), column 127 pin 127 (first bit)
        assert_eq!(pages[0].lines[0][15], 0x01);
        assert_eq!(pages[0].lines[1][0], 0x80);
    }
}
