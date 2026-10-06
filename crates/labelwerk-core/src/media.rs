//! Label media: tapes, die-cut and round labels of one printer model.
//!
//! The numbers come from Brother's own model definitions shipped with P-touch Editor (see
//! `tools/import-ptouch.py` and `data/models.json`). Sizes are in 0.1 mm, everything else in printer dots
//! at the model's resolution.

use serde::{Deserialize, Serialize};

use crate::model::Family;

pub fn mm_to_dots(mm: f32, dpi: u32) -> u32 {
    (mm * dpi as f32 / 25.4).round().max(0.0) as u32
}

pub fn dots_to_mm(dots: u32, dpi: u32) -> f32 {
    dots as f32 * 25.4 / dpi as f32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// Endless tape, the label length is chosen per print.
    Continuous,
    /// Pre-cut rectangular labels with a fixed size.
    DieCut,
    /// Pre-cut round labels.
    Round,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Media {
    /// Brother paper id (`nPaperSize`), stable across their software.
    pub id: u16,
    /// Brother's name: "62mm", "29mm x 90mm", "24mm Dia", "24 mm", "HS 5.8 mm".
    pub name: String,
    pub kind: Kind,
    /// Media type byte of the print information command (QL: 0x0A/0x0B, PT: 0x01 TZe, 0x11/0x17 tubes).
    pub media_type: u8,
    /// Physical width across the print head in 0.1 mm.
    pub width_tenth_mm: u16,
    /// Physical label length in 0.1 mm, 0 for continuous tape.
    pub length_tenth_mm: u16,
    /// Printable pins across the head.
    pub print_width: u32,
    /// Printable raster lines along the feed, 0 for continuous tape.
    pub print_length: u32,
    /// Unused pins on the pin-0 side of the head ("left margin" in Brother's tables).
    pub pins_left: u32,
    /// Unused pins on the far side of the head, at the start of each raster line ("right margin").
    pub pins_right: u32,
    /// Unprintable border around a die-cut label, in dots (across, along).
    pub offset_x: u32,
    pub offset_y: u32,
    /// Filled in from the model.
    #[serde(skip, default)]
    pub family: Family,
    #[serde(skip, default)]
    pub dpi: u32,
}

impl Media {
    pub fn width_mm(&self) -> f32 {
        self.width_tenth_mm as f32 / 10.0
    }

    pub fn length_mm(&self) -> f32 {
        self.length_tenth_mm as f32 / 10.0
    }

    pub fn is_tube(&self) -> bool {
        self.name.starts_with("HS")
    }

    /// Nominal size from the name in mm ("62mm x 29mm" -> 62, 29; "24mm Dia" -> 24, 24; "3.5 mm" -> 3.5, 0).
    pub fn nominal_mm(&self) -> (f32, f32) {
        let mut nums = self
            .name
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .filter_map(|s| s.trim_matches('.').parse::<f32>().ok());
        let w = nums.next().unwrap_or(0.0);
        let l = match self.kind {
            Kind::Continuous => 0.0,
            Kind::Round => w,
            Kind::DieCut => nums.next().unwrap_or(0.0),
        };
        (w, l)
    }

    /// Width and length in whole millimetres for the print information command, as Brother's drivers send
    /// them. A few differ from the label's name: QL 103 mm is 104, 60 x 86 mm is 60 x 87 (both documented),
    /// 102 x 152 mm is 102 x 153 (QL-1100 driver; the reference says 152), PT 3.5 mm tape is 4.
    pub fn status_size_mm(&self) -> (u8, u8) {
        let (w, l) = self.nominal_mm();
        let (w, l) = (w.round() as u8, l.round() as u8);
        match (self.family, self.id) {
            (Family::Ql, 265 | 385) => (104, l),
            (Family::Ql, 383) => (w, 87),
            (Family::Ql, 366) => (w, 153),
            _ => (w, l),
        }
    }

    /// Short English label: "62 mm endless", "62 × 29 mm", "Ø 24 mm", "24 mm tape", "HS 5.8 mm tube".
    pub fn label(&self) -> String {
        let (w, l) = self.nominal_mm();
        let w = fmt_mm(w);
        match (self.family, self.kind) {
            (Family::Pt, _) if self.is_tube() => format!("HS {w} mm tube"),
            (Family::Pt, _) => format!("{w} mm tape"),
            (_, Kind::Continuous) => format!("{w} mm endless"),
            (_, Kind::DieCut) => format!("{w} × {} mm", fmt_mm(l)),
            (_, Kind::Round) => format!("Ø {w} mm"),
        }
    }

    /// Stable key for settings and the CLI ("62", "62x29", "d24", "hs5.8").
    pub fn key(&self) -> String {
        let (w, l) = self.nominal_mm();
        let w = fmt_mm(w);
        match self.kind {
            Kind::Continuous if self.is_tube() => format!("hs{w}"),
            Kind::Continuous => w,
            Kind::DieCut => format!("{w}x{}", fmt_mm(l)),
            Kind::Round => format!("d{w}"),
        }
    }

    /// Normalise user input like "62 × 29 mm", "Ø24", "HS 5.8" to a key.
    pub fn normalize_key(input: &str) -> String {
        input.trim().to_lowercase().replace(' ', "").replace('×', "x").replace('ø', "d").replace("mm", "").replace(',', ".")
    }
}

fn fmt_mm(v: f32) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches(".0").to_string()
}
