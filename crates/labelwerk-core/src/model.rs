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
    /// No reference for this model; protocol from its family and open-source drivers.
    Assumed,
    /// Speaks another protocol (or none we know); printing is refused.
    Unsupported,
}

/// Command that switches the printer into raster mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeCommand {
    /// `ESC i a 01`
    DynamicMode,
    /// `ESC i R 01`, older PT models (as ptouch-print sends it)
    GraphicsMode,
    /// The printer only knows raster mode.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    /// Zero bytes sent first to flush a half-received job.
    pub invalidate: usize,
    pub raster: RasterCommand,
    pub mode: ModeCommand,
    /// Answers the status request `ESC i S`.
    pub status: bool,
    /// Print information command `ESC i z`, and the bits of its valid flag (n1).
    pub info: bool,
    pub info_flags: u8,
    /// n9 of `ESC i z` is 2 on the last page (PT-P900 family and the PT-D460BT group).
    pub last_page_flag: bool,
    /// Has an automatic cutter (`ESC i M` bit 6).
    pub cutter: bool,
    /// `ESC i A n` (cut every n labels).
    pub cut_every: bool,
    /// `ESC i K` expanded/advanced mode.
    pub expanded: bool,
    /// TIFF/PackBits compression (`M 0x02`) over USB.
    pub compression: bool,
    /// PT-D410/D460BT/D610BT/E310BT/E560BT: `ESC i d 01 00 4D 00` instead of margin and compression, and no
    /// `ESC i K` (ptouch-print; Brother publishes no reference for these).
    pub d460bt_magic: bool,
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
    protocol_for("", Family::Ql)
}

/// Dialect per model, from Brother's Raster Command References and, where Brother published none, from
/// ptouch-print and brother_ql. Sources per model in `research/protocol-table.md`.
fn protocol_for(name: &str, family: Family) -> Protocol {
    use ModeCommand::*;
    use Support::*;
    let ql = Protocol {
        invalidate: 200,
        raster: RasterCommand::Lower,
        mode: DynamicMode,
        status: true,
        info: true,
        // printer recovery, media type, width, length
        info_flags: 0x8E,
        last_page_flag: false,
        cutter: true,
        cut_every: true,
        expanded: true,
        compression: true,
        d460bt_magic: false,
        support: Documented,
    };
    // PT: Brother's drivers validate only the tape width (n1 = 0x84), so a non-laminated tape of the right
    // width is not rejected as "wrong media".
    let pt = Protocol { invalidate: 100, raster: RasterCommand::Upper, info_flags: 0x84, ..ql };
    // QL generation of 2011 (Raster Command Reference "QL series" v6.00): no ESC i A
    let ql_legacy = Protocol { cut_every: false, ..ql };
    let d460bt = Protocol {
        mode: DynamicMode,
        last_page_flag: true,
        compression: false,
        expanded: false,
        cut_every: false,
        d460bt_magic: true,
        support: Assumed,
        ..pt
    };
    let p900 = Protocol { invalidate: 200, last_page_flag: true, ..pt };
    match name {
        "QL-1100" => Protocol { invalidate: 400, support: Verified, ..ql },
        "QL-1110NWB" | "QL-1115NWB" => Protocol { invalidate: 400, ..ql },
        "QL-800" => Protocol { invalidate: 400, compression: false, ..ql },
        "QL-810W" | "QL-820NWB" => Protocol { invalidate: 400, ..ql },
        "QL-710W" | "QL-720NW" => ql,
        "QL-600" => Protocol { compression: false, ..ql },
        "QL-500" => Protocol { mode: None, compression: false, expanded: false, cutter: false, ..ql_legacy },
        "QL-550" => Protocol { mode: None, compression: false, expanded: false, ..ql_legacy },
        "QL-560" | "QL-570" | "QL-700" => Protocol { mode: None, compression: false, ..ql_legacy },
        // compression only over the serial port
        "QL-650TD" => Protocol { compression: false, ..ql_legacy },
        "QL-580N" | "QL-1060N" => ql_legacy,
        "QL-1050" => Protocol { invalidate: 350, ..ql_legacy },
        "PT-P710BT" => Protocol { cut_every: false, ..pt },
        "PT-E550W" | "PT-P750W" => Protocol { status: false, ..pt },
        "PT-H500" | "PT-P700" | "PT-E500" => Protocol { cut_every: false, ..pt },
        "PT-P900" | "PT-P900W" | "PT-P950NW" => p900,
        "PT-P910BT" => Protocol { expanded: false, ..p900 },
        "PT-D410" | "PT-D460BT" | "PT-D610BT" | "PT-E310BT" | "PT-E560BT" => d460bt,
        "PT-D450" => Protocol { compression: false, cut_every: false, support: Assumed, ..pt },
        "PT-2430PC" | "PT-2700" | "PT-2730" => Protocol {
            mode: GraphicsMode,
            info: false,
            compression: false,
            cut_every: false,
            expanded: false,
            support: Assumed,
            ..pt
        },
        "PT-P300BT" => Protocol { invalidate: 64, cut_every: false, support: Assumed, ..pt },
        // a different raster dialect (ESC i R, ESC i c) or no raster mode at all
        "PT-9500PC" | "PT-9600" | "PT-9700PC" | "PT-9800PCN" | "PT-3600" | "PT-18NR" | "PT-18R" | "PT-N25BT" => {
            Protocol { support: Unsupported, ..pt }
        }
        _ => match family {
            Family::Ql => Protocol { support: Assumed, ..ql },
            Family::Pt if name.starts_with("PT-E9") || name.starts_with("PT-E8") || name.starts_with("PT-D8") => {
                Protocol { support: Assumed, ..p900 }
            }
            Family::Pt => Protocol { cut_every: false, support: Assumed, ..pt },
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

    /// The model named in a USB product string ("PT-P750W"), for printers that do not answer `ESC i S`.
    pub fn by_product(product: &str) -> Option<&'static Model> {
        models().iter().find(|m| product.split_whitespace().any(|w| w.eq_ignore_ascii_case(&m.name)))
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
                    // the QL-700 and QL-800 generations report 0x4A / 0x4B
                    code == media_type & !0x40 && w == width_mm && (m.kind == Kind::Continuous || l.abs_diff(length_mm) <= tolerance)
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
    fn ql800_generation_reports_new_media_codes() {
        let ql = Model::by_name("QL-820NWB").unwrap();
        assert_eq!(ql.media_from_status(0x4A, 62, 0).unwrap().key(), "62");
        assert_eq!(ql.media_from_status(0x4B, 62, 29).unwrap().key(), "62x29");
    }

    #[test]
    fn status_less_models_are_found_by_product_name() {
        let m = Model::by_product("PT-P750W").unwrap();
        assert!(!m.protocol.status);
        assert_eq!(Model::by_name("PT-9700PC").unwrap().protocol.support, Support::Unsupported);
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
