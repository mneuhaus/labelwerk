//! Byte-level comparison with Brother's own macOS driver.
//!
//! Each case writes a CUPS raster page with a known bitmap, runs it through `rastertobrotherQL1100` (the
//! filter CUPS uses for the QL-1100) and checks that our encoder produces the same commands and the same
//! head lines. Skipped when the driver is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

use labelwerk_core::protocol::{DecodedPage, decode_job, encode_job, lines_to_bitmap};
use labelwerk_core::{Bitmap, Kind, Media, Model, PrintOptions};

fn ql1100() -> &'static Model {
    Model::by_name("QL-1100").unwrap()
}

const FILTER: &str =
    "/Library/Printers/Brother/Filter/rastertobrotherQL1100.bundle/Contents/MacOS/rastertobrotherQL1100";
const PPD: &str = "/etc/cups/ppd/Brother_QL_1100.ppd";

fn driver_available() -> bool {
    let ok = Path::new(FILTER).exists() && Path::new(PPD).exists();
    if !ok {
        eprintln!("Brother QL-1100 driver not installed, skipping");
    }
    ok
}

/// The PPD keyword for a media ("62mm", "DC06", ...), found by its display name ("62 mm x 29 mm").
fn ppd_keyword(media: &Media) -> String {
    let display = media.name.replace("mm", " mm");
    let ppd = std::fs::read_to_string(PPD).unwrap();
    ppd.lines()
        .filter_map(|l| l.strip_prefix("*PageSize "))
        .find_map(|l| {
            let (key, rest) = l.split_once('/')?;
            let name = rest.split(':').next()?.trim();
            (name == display).then(|| key.to_string())
        })
        .unwrap_or_else(|| panic!("no PPD page size for {display}"))
}

/// Minimal CUPS raster v3 (little endian, uncompressed), 8-bit "K" colour space: 0 = white, 255 = black.
fn cups_raster(pages: &[&Bitmap], page_name: &str) -> Vec<u8> {
    let mut out = b"3SaR".to_vec();
    for page in pages {
        out.extend_from_slice(&page_header(page, page_name));
        out.extend(page.pixels.iter().map(|&p| p * 255));
    }
    out
}

fn page_header(page: &Bitmap, page_name: &str) -> Vec<u8> {
    let mut h = vec![0u8; 1796];
    let put = |h: &mut Vec<u8>, off: usize, v: u32| h[off..off + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut h, 276, 300); // HWResolution
    put(&mut h, 280, 300);
    let pts = |dots: u32| (dots as f32 * 72.0 / 300.0).round() as u32;
    put(&mut h, 340, 1); // NumCopies
    put(&mut h, 352, pts(page.width)); // PageSize
    put(&mut h, 356, pts(page.height));
    put(&mut h, 372, page.width); // cupsWidth
    put(&mut h, 376, page.height); // cupsHeight
    put(&mut h, 384, 8); // cupsBitsPerColor
    put(&mut h, 388, 8); // cupsBitsPerPixel
    put(&mut h, 392, page.width); // cupsBytesPerLine
    put(&mut h, 400, 3); // cupsColorSpace = K
    put(&mut h, 420, 1); // cupsNumColors
    h[1732..1732 + page_name.len()].copy_from_slice(page_name.as_bytes());
    h
}

fn run_filter(media: &Media, pages: &[&Bitmap], name: &str) -> Vec<DecodedPage> {
    let keyword = ppd_keyword(media);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("brother-filter");
    std::fs::create_dir_all(&dir).unwrap();
    let ras = dir.join(format!("{name}.ras"));
    std::fs::write(&ras, cups_raster(pages, &keyword)).unwrap();
    let out = Command::new(FILTER)
        .env("PPD", PPD)
        .args(["1", "labelwerk", name, "1", &format!("media={keyword}")])
        .arg(&ras)
        .output()
        .unwrap();
    assert!(!out.stdout.is_empty(), "filter produced nothing: {}", String::from_utf8_lossy(&out.stderr));
    std::fs::write(dir.join(format!("{name}.prn")), &out.stdout).unwrap();
    decode_job(ql1100(), &out.stdout).unwrap()
}

/// Features that make orientation, offsets and compression edge cases visible.
fn test_page(width: u32, height: u32) -> Bitmap {
    let mut b = Bitmap::new(width, height);
    // solid block in the top-left corner: catches mirroring and flipped feed direction
    for y in 0..40 {
        for x in 0..80 {
            b.set(x, y, true);
        }
    }
    // single pixels at both edges of the print area
    b.set(0, 60, true);
    b.set(width - 1, 61, true);
    // vertical bar at the right edge
    for y in 80..height - 10 {
        b.set(width - 3, y, true);
    }
    // worst case for PackBits: "1 00 00" pairs over the whole line
    let worst = 100.min(height - 1);
    for x in 0..width {
        b.set(x, worst, x % 24 < 8 && x % 24 != 0);
    }
    // noise
    let mut seed = 42u32;
    for y in (height / 2)..(height / 2 + 40).min(height) {
        for x in 0..width {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            b.set(x, y, seed >> 31 == 1);
        }
    }
    b
}

fn compare(media: &Media, page: &Bitmap, copies: u32, name: &str) {
    let pages: Vec<&Bitmap> = std::iter::repeat_n(page, copies as usize).collect();
    let theirs = run_filter(media, &pages, name);
    let opts = PrintOptions { margin_dots: Some(margin_of(&theirs[0])), ..PrintOptions::default() };
    let ours = decode_job(ql1100(), &encode_job(ql1100(), media, &pages, &opts).unwrap()).unwrap();
    assert_eq!(ours.len(), theirs.len(), "{name}: page count");
    for (i, (o, t)) in ours.iter().zip(&theirs).enumerate() {
        assert_eq!(o.commands, t.commands, "{name} page {i}: commands");
        assert_eq!(o.terminator, t.terminator, "{name} page {i}: terminator");
        assert_eq!(o.lines.len(), t.lines.len(), "{name} page {i}: line count");
        if o.lines != t.lines {
            let theirs_bitmap = lines_to_bitmap(ql1100(), media, &t.lines);
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("brother-filter");
            std::fs::write(dir.join(format!("{name}-theirs.png")), theirs_bitmap.to_png()).unwrap();
            std::fs::write(dir.join(format!("{name}-ours.png")), page.to_png()).unwrap();
            let first = o.lines.iter().zip(&t.lines).position(|(a, b)| a != b).unwrap();
            panic!(
                "{name} page {i}: line {first} differs\n ours  {:02x?}\n theirs {:02x?}",
                o.lines[first], t.lines[first]
            );
        }
    }
}

/// Brother's driver may pick its own feed margin; take it from its output for the comparison.
fn margin_of(page: &DecodedPage) -> u16 {
    page.commands
        .iter()
        .find(|c| c.starts_with(&[0x1B, b'i', b'd']))
        .map(|c| u16::from_le_bytes([c[3], c[4]]))
        .unwrap_or(0)
}

#[test]
fn continuous_62mm_matches_driver() {
    if !driver_available() {
        return;
    }
    let media = ql1100().media_by_key("62").unwrap();
    compare(media, &test_page(media.print_width, 400), 1, "c62");
}

#[test]
fn die_cut_labels_match_driver() {
    if !driver_available() {
        return;
    }
    for key in ["62x29", "29x90", "102x152", "d24"] {
        let media = ql1100().media_by_key(key).unwrap();
        compare(media, &test_page(media.print_width, media.print_length), 1, key);
    }
}

#[test]
fn every_media_matches_driver() {
    if !driver_available() {
        return;
    }
    for media in &ql1100().media {
        // Brother's CUPS driver deviates from the Raster Command Reference and P-touch Editor's model data
        // here: 23x23 one pin off, 60x86 three extra lines, 12 mm round with a 3 mm feed margin although
        // die-cut labels take none. We follow the reference.
        if matches!(media.key().as_str(), "23x23" | "60x86" | "d12") {
            continue;
        }
        let height = if media.kind == Kind::Continuous { 320 } else { media.print_length };
        compare(media, &test_page(media.print_width, height), 1, &format!("all-{}", media.key()));
    }
}

#[test]
fn copies_match_driver() {
    if !driver_available() {
        return;
    }
    let media = ql1100().media_by_key("62x29").unwrap();
    compare(media, &test_page(media.print_width, media.print_length), 3, "copies");
}

/// Diagnostic: where Brother's driver puts page column 0 for every media (`--ignored --nocapture`).
#[test]
#[ignore]
fn report_driver_pin_offsets() {
    if !driver_available() {
        return;
    }
    for media in &ql1100().media {
        let height = if media.kind == Kind::Continuous { 320 } else { media.print_length };
        let mut page = Bitmap::new(media.print_width, height);
        page.set(0, 0, true);
        page.set(media.print_width - 1, 1, true);
        let theirs = run_filter(media, &[&page], &format!("pins-{}", media.key()));
        let bit = |line: &Vec<u8>| (0..1296).find(|b| line[b / 8] & (0x80 >> (b % 8)) != 0);
        let (b0, b1) = (bit(&theirs[0].lines[0]), bit(&theirs[0].lines[1]));
        let pin0 = b0.map(|b| 1295 - b as i32);
        let pin_last = b1.map(|b| 1295 - b as i32);
        println!(
            "{:>8} table pins_left {:>4} width {:>4} | driver col0 -> pin {:?}, last col -> pin {:?}, lines {}",
            media.key(),
            media.pins_left,
            media.print_width,
            pin0,
            pin_last,
            theirs[0].lines.len()
        );
    }
}
