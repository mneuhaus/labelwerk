//! What the loaded tape looks like: colours reported by PT printers (status bytes 24 and 25, Raster Command
//! Reference PT-E550W/P750W/P710BT, tables 8 and 9). QL paper is white with black print.

use labelwerk_core::Family;

use crate::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub tape: [u8; 3],
    pub ink: [u8; 3],
    /// "White / Black", "Weiß / Schwarz"
    pub tape_name: &'static str,
    pub ink_name: &'static str,
}

const PAPER_TAPE: [u8; 3] = [252, 252, 249];
const PAPER_INK: [u8; 3] = [20, 20, 20];

/// Plain white paper, shown when no printer reports actual tape colours.
pub fn paper() -> Look {
    Look { tape: PAPER_TAPE, ink: PAPER_INK, tape_name: tr!("White", "Weiß"), ink_name: tr!("Black", "Schwarz") }
}

fn tape(id: u8) -> Option<([u8; 3], &'static str)> {
    Some(match id {
        0x01 | 0x90 => ([250, 250, 246], tr!("White", "Weiß")),
        0x02 => ([226, 226, 222], tr!("Special color", "Sonderfarbe")),
        0x03 | 0x09 => ([226, 233, 236], tr!("Clear", "Transparent")),
        0x04 | 0x31 => ([201, 48, 44], tr!("Red", "Rot")),
        0x05 | 0x30 => ([44, 98, 176], tr!("Blue", "Blau")),
        0x06 | 0x60 | 0x91 => ([243, 208, 47], tr!("Yellow", "Gelb")),
        0x07 => ([62, 156, 85], tr!("Green", "Grün")),
        0x08 => ([30, 30, 30], tr!("Black", "Schwarz")),
        0x20 => ([242, 242, 238], tr!("Matte white", "Matt weiß")),
        0x21 => ([228, 232, 234], tr!("Matte clear", "Matt transparent")),
        0x22 => ([199, 202, 205], tr!("Matte silver", "Matt silber")),
        0x23 => ([205, 174, 98], tr!("Satin gold", "Satin gold")),
        0x24 => ([191, 195, 199], tr!("Satin silver", "Satin silber")),
        0x40 => ([255, 122, 46], tr!("Neon orange", "Neon orange")),
        0x41 => ([233, 242, 74], tr!("Neon yellow", "Neon gelb")),
        0x50 => ([224, 106, 154], tr!("Berry pink", "Beerenpink")),
        0x51 => ([201, 205, 208], tr!("Light gray", "Hellgrau")),
        0x52 => ([169, 212, 106], tr!("Lime green", "Limettengrün")),
        0x61 => ([242, 154, 192], tr!("Pink", "Pink")),
        0x62 => ([110, 165, 222], tr!("Light blue", "Hellblau")),
        0x70 => ([244, 244, 240], tr!("White (tube)", "Weiß (Schlauch)")),
        _ => return None,
    })
}

fn ink(id: u8) -> Option<([u8; 3], &'static str)> {
    Some(match id {
        0x01 => ([255, 255, 255], tr!("White", "Weiß")),
        0x04 => ([200, 16, 46], tr!("Red", "Rot")),
        0x05 | 0x62 => ([31, 78, 156], tr!("Blue", "Blau")),
        0x08 => ([17, 17, 17], tr!("Black", "Schwarz")),
        0x0A => ([184, 145, 47], tr!("Gold", "Gold")),
        0x02 => ([51, 51, 51], tr!("Special color", "Sonderfarbe")),
        _ => return None,
    })
}

/// The look of the loaded media; `colors` = (tape colour id, text colour id) from a PT status.
pub fn look(family: Family, colors: Option<(u8, u8)>) -> Look {
    match (family, colors) {
        (Family::Pt, Some((t, i))) => {
            let (tape, tape_name) = tape(t).unwrap_or((PAPER_TAPE, tr!("Unknown", "Unbekannt")));
            let (ink, ink_name) = ink(i).unwrap_or((PAPER_INK, tr!("Unknown", "Unbekannt")));
            Look { tape, ink, tape_name, ink_name }
        }
        _ => paper(),
    }
}
