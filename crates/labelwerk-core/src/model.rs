//! Printer models: facts from P-touch Editor (`data/models.json`) plus the raster-protocol dialect each
//! model speaks, from Brother's Raster Command References.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::media::{Kind, Media};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Family {
    /// QL: die-cut and continuous paper labels, 300 dpi.
    #[default]
    #[serde(rename = "QL")]
    Ql,
    /// PT / P-touch: laminated TZe tapes and heat-shrink tubes, 180 or 360 dpi.
    #[serde(rename = "PT")]
    Pt,
}

/// How a raster line is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterCommand {
    /// `g 0x00 n data` (QL)
    Lower,
    /// `G n1 n2 data` with a 16-bit length (PT)
    Upper,
}

/// How far a model's support is backed by evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Support {
    /// Output compared with Brother's own driver or printed on the real device.
    Verified,
    /// Implemented from Brother's Raster Command Reference for this model, not yet printed.
    Documented,
    /// Same family and print head as documented models, but no reference for this exact model.
    Assumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    /// Zero bytes sent first to flush a half-received job.
    pub invalidate: usize,
    pub raster: RasterCommand,
    /// TIFF/PackBits compression (`M 0x02`).
    pub compression: bool,
    /// `ESC i A n` (cut every n labels).
    pub cut_every: bool,
    /// Bits of the print information command's valid flag (`ESC i z` n1).
    pub info_flags: u8,
    pub support: Support,
}

#[derive(Debug, Deserialize)]
pub struct Model {
    pub name: String,
    pub family: Family,
    pub series_code: u8,
    pub model_code: u8,
    pub head_pins: u32,
    pub dpi: u32,
    /// Smallest feed margin on continuous tape, in dots.
    pub min_margin_dots: u16,
    /// Feed margin P-touch Editor uses by default, in dots.
    pub default_margin_dots: u16,
    pub min_length_tenth_mm: u32,
    pub max_length_tenth_mm: u32,
    pub max_copies: u32,
    pub media: Vec<Media>,
    #[serde(skip, default = "placeholder_protocol")]
    pub protocol: Protocol,
}

fn placeholder_protocol() -> Protocol {
    Protocol {
        invalidate: 200,
        raster: RasterCommand::Lower,
        compression: true,
        cut_every: true,
        info_flags: 0x8E,
        support: Support::Assumed,
    }
}

/// Dialect per model, from the Raster Command References (see `research/protocol-table.md`).
fn protocol_for(name: &str, family: Family) -> Protocol {
    let ql = |invalidate, support| Protocol {
        invalidate,
        raster: RasterCommand::Lower,
        compression: true,
        cut_every: true,
        // printer recovery, media type, width, length
        info_flags: 0x8E,
        support,
    };
    // PT: Brother's own drivers only validate the tape width (n1 = 0x84), so a non-laminated tape of the
    // right width is not rejected as "wrong media".
    let pt = |cut_every, support| Protocol {
        invalidate: 100,
        raster: RasterCommand::Upper,
        compression: true,
        cut_every,
        info_flags: 0x84,
        support,
    };
    match name {
        "QL-1100" | "QL-1110NWB" | "QL-1115NWB" => ql(400, Support::Verified),
        "PT-P710BT" => pt(false, Support::Documented),
        "PT-E550W" | "PT-P750W" => pt(true, Support::Documented),
        _ => match family {
            Family::Ql => ql(200, Support::Assumed),
            Family::Pt => pt(true, Support::Assumed),
        },
    }
}

static MODELS: OnceLock<Vec<Model>> = OnceLock::new();

#[derive(Deserialize)]
struct Database {
    models: Vec<Model>,
}

/// Every QL and PT model P-touch Editor knows.
pub fn models() -> &'static [Model] {
    MODELS.get_or_init(|| {
        let db: Database = serde_json::from_str(include_str!("../data/models.json")).expect("bundled model data");
        db.models
            .into_iter()
            .map(|mut m| {
                m.protocol = protocol_for(&m.name, m.family);
                for media in &mut m.media {
                    media.family = m.family;
                    media.dpi = m.dpi;
                }
                m
            })
            .collect()
    })
}

/// Models are unique by name.
impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Model {}

impl Model {
    pub fn by_name(name: &str) -> Option<&'static Model> {
        let name = name.trim().trim_start_matches("Brother ").to_ascii_uppercase();
        models().iter().find(|m| m.name.to_ascii_uppercase() == name)
    }

    /// The model behind a status reply (bytes 3 and 4).
    pub fn by_codes(series_code: u8, model_code: u8) -> Option<&'static Model> {
        models().iter().find(|m| m.series_code == series_code && m.model_code == model_code)
    }

    pub fn line_bytes(&self) -> usize {
        self.head_pins.div_ceil(8) as usize
    }

    pub fn media_by_key(&'static self, key: &str) -> Option<&'static Media> {
        let key = Media::normalize_key(key);
        self.media.iter().find(|m| m.key() == key)
    }

    pub fn media_by_id(&'static self, id: u16) -> Option<&'static Media> {
        self.media.iter().find(|m| m.id == id)
    }

    /// The media the printer reports as loaded (status bytes 10, 11 and 17).
    pub fn media_from_status(&'static self, media_type: u8, width_mm: u8, length_mm: u8) -> Option<&'static Media> {
        match self.family {
            Family::Ql => {
                // Die-cut lengths match within 1 mm because Brother's own numbers disagree by one for some labels.
                let fits = |m: &&Media, tolerance: u8| {
                    let (w, l) = m.status_size_mm();
                    let code = if m.kind == Kind::Continuous { 0x0A } else { 0x0B };
                    code == media_type && w == width_mm && (m.kind == Kind::Continuous || l.abs_diff(length_mm) <= tolerance)
                };
                self.media.iter().find(|m| fits(m, 0)).or_else(|| self.media.iter().find(|m| fits(m, 1)))
            }
            Family::Pt => {
                // laminated and non-laminated TZe share one geometry; tubes come in 2:1 (0x11) and 3:1 (0x17)
                let tube = matches!(media_type, 0x11 | 0x17);
                let candidates = self.media.iter().filter(|m| {
                    m.is_tube() == tube && (!tube || m.media_type == media_type || m.media_type == 0)
                });
                candidates
                    .map(|m| (m, (m.width_mm() - width_mm as f32).abs()))
                    .filter(|(_, d)| *d <= 1.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(m, _)| m)
            }
        }
    }

    /// Shortest and longest continuous label in raster lines.
    pub fn length_limits(&self) -> (u32, u32) {
        let to_dots = |tenth: u32| crate::media::mm_to_dots(tenth as f32 / 10.0, self.dpi);
        let min = if self.min_length_tenth_mm > 0 { to_dots(self.min_length_tenth_mm) } else { to_dots(254) };
        let max = if self.max_length_tenth_mm > 0 { to_dots(self.max_length_tenth_mm) } else { to_dots(10_000) };
        (min.max(1), max)
    }

    pub fn default_model() -> &'static Model {
        Model::by_name("QL-1100").expect("QL-1100 in model data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_loads_and_media_fill_the_head() {
        assert!(models().len() > 40);
        for m in models() {
            for media in &m.media {
                // a few tiny tapes on older models are not centred on the head; the sum must still fit
                assert!(media.pins_left + media.print_width + media.pins_right <= m.head_pins + 1, "{} {}", m.name, media.name);
                assert_eq!(media.dpi, m.dpi);
            }
        }
    }

    #[test]
    fn identifies_models_from_status_codes() {
        assert_eq!(Model::by_codes(0x34, 0x43).unwrap().name, "QL-1100");
        let p710 = Model::by_codes(0x30, 0x76).unwrap();
        assert_eq!(p710.name, "PT-P710BT");
        assert_eq!((p710.head_pins, p710.dpi, p710.line_bytes()), (128, 180, 16));
    }

    #[test]
    fn ql1100_media_match_the_status() {
        let ql = Model::by_name("QL-1100").unwrap();
        assert_eq!(ql.media_from_status(0x0A, 62, 0).unwrap().key(), "62");
        assert_eq!(ql.media_from_status(0x0B, 62, 29).unwrap().key(), "62x29");
        assert_eq!(ql.media_from_status(0x0A, 104, 0).unwrap().key(), "103");
        assert_eq!(ql.media_from_status(0x0B, 60, 87).unwrap().key(), "60x86");
        assert_eq!(ql.media_from_status(0x0B, 102, 152).unwrap().key(), "102x152");
        assert_eq!(ql.media_from_status(0x0B, 24, 24).unwrap().kind, Kind::Round);
        assert!(ql.media_from_status(0x0B, 62, 31).is_none());
    }

    #[test]
    fn p710bt_tapes_match_the_status() {
        let pt = Model::by_name("PT-P710BT").unwrap();
        let tape = pt.media_from_status(0x01, 24, 0).unwrap();
        assert_eq!((tape.key().as_str(), tape.print_width, tape.pins_left), ("24", 128, 0));
        assert_eq!(pt.media_from_status(0x03, 12, 0).unwrap().print_width, 70);
        assert_eq!(pt.media_from_status(0x01, 4, 0).unwrap().key(), "3.5");
        assert!(pt.media_from_status(0x11, 6, 0).unwrap().is_tube());
    }

    #[test]
    fn keys_round_trip() {
        for m in models() {
            for media in &m.media {
                let found = m.media_by_key(&media.key()).unwrap();
                assert_eq!(found.key(), media.key(), "{} {}", m.name, media.name);
            }
        }
        let ql = Model::by_name("QL-1100").unwrap();
        assert_eq!(ql.media_by_key("62 × 29 mm").unwrap().id, 274);
    }
}
