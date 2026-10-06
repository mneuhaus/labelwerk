//! Label media of the QL-1100 family.
//!
//! Numbers come from Brother's own model definition shipped with P-touch Editor 5.3
//! (`BRLBXWrapperMac.framework/.../ptd.bundle/bsq110ad.json`, identical for the QL-1110NWB) and agree with
//! the "Raster Command Reference QL-1100/1110NWB/1115NWB" (v1.00). Lengths in 0.1 mm, everything else in
//! printer dots (300 dpi). The "x2/x3/x4" split-printing entries of P-touch Editor are left out.

use serde::{Deserialize, Serialize};

pub const DPI: f32 = 300.0;

pub fn mm_to_dots(mm: f32) -> u32 {
    (mm * DPI / 25.4).round().max(0.0) as u32
}

pub fn dots_to_mm(dots: u32) -> f32 {
    dots as f32 * 25.4 / DPI
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

impl Kind {
    /// Media type byte used by the print information command and reported in the status.
    pub fn code(self) -> u8 {
        match self {
            Kind::Continuous => 0x0A,
            Kind::DieCut | Kind::Round => 0x0B,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Media {
    /// Brother paper id (`nPaperSize`), stable across their software.
    pub id: u16,
    /// Brother's name, also the CUPS `media=` keyword family ("62mm", "29mm x 90mm", "24mm Dia").
    pub brother_name: &'static str,
    pub kind: Kind,
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
    /// Unused pins on the far side of the head, i.e. at the start of each raster line ("right margin").
    pub pins_right: u32,
    /// Unprintable border around a die-cut label, in dots (across, along).
    pub offset_x: u32,
    pub offset_y: u32,
}

impl Media {
    pub fn width_mm(&self) -> f32 {
        self.width_tenth_mm as f32 / 10.0
    }

    pub fn length_mm(&self) -> f32 {
        self.length_tenth_mm as f32 / 10.0
    }

    /// Nominal size from the name ("62mm x 29mm" -> 62, 29; "24mm Dia" -> 24, 24; "62mm" -> 62, 0).
    pub fn nominal_mm(&self) -> (u8, u8) {
        let mut nums = self.brother_name.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok());
        let w = nums.next().unwrap_or(0);
        let l = match self.kind {
            Kind::Continuous => 0,
            Kind::Round => w,
            Kind::DieCut => nums.next().unwrap_or(0),
        };
        (w, l)
    }

    /// Width and length in whole millimetres for the print information command, as Brother's driver sends
    /// them. A few differ from the label's name: 103 mm is 104, 60 x 86 mm is 60 x 87 (both documented) and
    /// 102 x 152 mm is 102 x 153 (driver; the reference says 152).
    pub fn status_size_mm(&self) -> (u8, u8) {
        match (self.id, self.nominal_mm()) {
            (265 | 385, (_, l)) => (104, l),
            (383, (w, _)) => (w, 87),
            (366, (w, _)) => (w, 153),
            (_, size) => size,
        }
    }

    /// Short human label: "62 mm endless", "62 × 29 mm", "Ø 24 mm".
    pub fn label(&self) -> String {
        let (w, l) = self.nominal_mm();
        match self.kind {
            Kind::Continuous => format!("{w} mm endless"),
            Kind::DieCut => format!("{w} × {l} mm"),
            Kind::Round => format!("Ø {w} mm"),
        }
    }

    /// Stable key for settings and the CLI ("62", "62x29", "d24").
    pub fn key(&self) -> String {
        let (w, l) = self.nominal_mm();
        match self.kind {
            Kind::Continuous => format!("{w}"),
            Kind::DieCut => format!("{w}x{l}"),
            Kind::Round => format!("d{w}"),
        }
    }

    pub fn by_id(id: u16) -> Option<&'static Media> {
        MEDIA.iter().find(|m| m.id == id)
    }

    pub fn by_key(key: &str) -> Option<&'static Media> {
        let key = key.trim().to_lowercase().replace(' ', "").replace('×', "x").replace('ø', "d").replace("mm", "");
        MEDIA.iter().find(|m| m.key() == key)
    }

    /// The media the printer reports as loaded (status bytes 10, 11, 17).
    /// Die-cut lengths match within 1 mm because Brother's own numbers disagree by one for some labels.
    pub fn from_status(media_type: u8, width_mm: u8, length_mm: u8) -> Option<&'static Media> {
        let fits = |m: &&Media, tolerance: u8| {
            let (w, l) = m.status_size_mm();
            m.kind.code() == media_type
                && w == width_mm
                && (m.kind == Kind::Continuous || l.abs_diff(length_mm) <= tolerance)
        };
        MEDIA.iter().find(|m| fits(m, 0)).or_else(|| MEDIA.iter().find(|m| fits(m, 1)))
    }
}

#[allow(clippy::too_many_arguments)]
const fn m(
    id: u16,
    brother_name: &'static str,
    kind: Kind,
    width_tenth_mm: u16,
    length_tenth_mm: u16,
    print_width: u32,
    print_length: u32,
    pins_left: u32,
    pins_right: u32,
    offset_x: u32,
    offset_y: u32,
) -> Media {
    Media {
        id,
        brother_name,
        kind,
        width_tenth_mm,
        length_tenth_mm,
        print_width,
        print_length,
        pins_left,
        pins_right,
        offset_x,
        offset_y,
    }
}

/// QL-1100 / QL-1110NWB / QL-1115NWB media, in the order P-touch Editor lists them.
pub static MEDIA: &[Media] = &[
    m(259, "62mm", Kind::Continuous, 620, 0, 696, 0, 544, 56, 18, 35),
    m(260, "102mm", Kind::Continuous, 1016, 0, 1164, 0, 76, 56, 18, 35),
    m(265, "103mm", Kind::Continuous, 1036, 0, 1200, 0, 58, 38, 12, 35),
    m(262, "50mm", Kind::Continuous, 500, 0, 554, 0, 686, 56, 18, 35),
    m(261, "54mm", Kind::Continuous, 538, 0, 590, 0, 662, 44, 23, 35),
    m(264, "38mm", Kind::Continuous, 380, 0, 413, 0, 827, 56, 18, 35),
    m(258, "29mm", Kind::Continuous, 290, 0, 306, 0, 940, 50, 18, 35),
    m(257, "12mm", Kind::Continuous, 120, 0, 106, 0, 1116, 74, 18, 35),
    m(269, "17mm x 54mm", Kind::DieCut, 170, 539, 165, 566, 1087, 44, 18, 35),
    m(270, "17mm x 87mm", Kind::DieCut, 170, 869, 165, 956, 1087, 44, 18, 35),
    m(370, "23mm x 23mm", Kind::DieCut, 230, 230, 236, 202, 975, 85, 18, 35),
    m(358, "29mm x 42mm", Kind::DieCut, 290, 419, 306, 425, 940, 50, 18, 35),
    m(271, "29mm x 90mm", Kind::DieCut, 290, 898, 306, 991, 940, 50, 18, 35),
    m(272, "38mm x 90mm", Kind::DieCut, 380, 898, 413, 991, 827, 56, 18, 35),
    m(367, "39mm x 48mm", Kind::DieCut, 390, 478, 425, 495, 821, 50, 18, 35),
    m(374, "52mm x 29mm", Kind::DieCut, 520, 289, 578, 271, 674, 44, 18, 35),
    m(383, "60mm x 86mm", Kind::DieCut, 600, 868, 672, 954, 556, 68, 18, 35),
    m(274, "62mm x 29mm", Kind::DieCut, 620, 289, 696, 271, 544, 56, 18, 35),
    m(275, "62mm x 100mm", Kind::DieCut, 620, 998, 696, 1109, 544, 56, 18, 35),
    m(365, "102mm x 51mm", Kind::DieCut, 1016, 505, 1164, 526, 76, 56, 18, 36),
    m(366, "102mm x 152mm", Kind::DieCut, 1016, 1528, 1164, 1660, 76, 56, 18, 72),
    m(385, "103mm x 164mm", Kind::DieCut, 1036, 1643, 1200, 1822, 58, 38, 12, 59),
    m(362, "12mm Dia", Kind::Round, 120, 120, 94, 94, 1046, 156, 24, 24),
    m(363, "24mm Dia", Kind::Round, 240, 240, 236, 236, 975, 85, 24, 24),
    m(273, "58mm Dia", Kind::Round, 583, 583, 618, 618, 584, 94, 35, 35),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_media_fills_the_whole_head() {
        for m in MEDIA {
            assert_eq!(m.pins_left + m.print_width + m.pins_right, 1296, "{}", m.brother_name);
        }
    }

    #[test]
    fn status_lookup_finds_loaded_media() {
        assert_eq!(Media::from_status(0x0A, 62, 0).unwrap().id, 259);
        assert_eq!(Media::from_status(0x0B, 62, 29).unwrap().id, 274);
        assert_eq!(Media::from_status(0x0A, 104, 0).unwrap().id, 265);
        assert_eq!(Media::from_status(0x0B, 60, 87).unwrap().id, 383);
        assert_eq!(Media::from_status(0x0B, 24, 24).unwrap().kind, Kind::Round);
        assert_eq!(Media::from_status(0x0B, 102, 152).unwrap().id, 366);
        assert_eq!(Media::from_status(0x0B, 102, 153).unwrap().id, 366);
        assert!(Media::from_status(0x0B, 62, 31).is_none());
    }

    #[test]
    fn keys_round_trip() {
        for m in MEDIA {
            assert_eq!(Media::by_key(&m.key()).unwrap().id, m.id);
        }
        assert_eq!(Media::by_key("62 × 29 mm").unwrap().id, 274);
        assert_eq!(Media::by_key("62mm").unwrap().id, 259);
    }
}
