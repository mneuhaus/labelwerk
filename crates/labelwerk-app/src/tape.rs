//! What the loaded tape looks like: colours reported by PT printers (status bytes 24 and 25, Raster Command
//! Reference PT-E550W/P750W/P710BT, tables 8 and 9). QL paper is white with black print.

use labelwerk_core::Family;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub tape: [u8; 3],
    pub ink: [u8; 3],
    /// "Weiß / Schwarz"
    pub tape_name: &'static str,
    pub ink_name: &'static str,
}

pub const PAPER: Look = Look { tape: [252, 252, 249], ink: [20, 20, 20], tape_name: "Weiß", ink_name: "Schwarz" };

fn tape(id: u8) -> Option<([u8; 3], &'static str)> {
    Some(match id {
        0x01 | 0x90 => ([250, 250, 246], "Weiß"),
        0x02 => ([226, 226, 222], "Sonderfarbe"),
        0x03 | 0x09 => ([226, 233, 236], "Transparent"),
        0x04 | 0x31 => ([201, 48, 44], "Rot"),
        0x05 | 0x30 => ([44, 98, 176], "Blau"),
        0x06 | 0x60 | 0x91 => ([243, 208, 47], "Gelb"),
        0x07 => ([62, 156, 85], "Grün"),
        0x08 => ([30, 30, 30], "Schwarz"),
        0x20 => ([242, 242, 238], "Matt weiß"),
        0x21 => ([228, 232, 234], "Matt transparent"),
        0x22 => ([199, 202, 205], "Matt silber"),
        0x23 => ([205, 174, 98], "Satin gold"),
        0x24 => ([191, 195, 199], "Satin silber"),
        0x40 => ([255, 122, 46], "Neon orange"),
        0x41 => ([233, 242, 74], "Neon gelb"),
        0x50 => ([224, 106, 154], "Beerenpink"),
        0x51 => ([201, 205, 208], "Hellgrau"),
        0x52 => ([169, 212, 106], "Limettengrün"),
        0x61 => ([242, 154, 192], "Pink"),
        0x62 => ([110, 165, 222], "Hellblau"),
        0x70 => ([244, 244, 240], "Weiß (Schlauch)"),
        _ => return None,
    })
}

fn ink(id: u8) -> Option<([u8; 3], &'static str)> {
    Some(match id {
        0x01 => ([255, 255, 255], "Weiß"),
        0x04 => ([200, 16, 46], "Rot"),
        0x05 | 0x62 => ([31, 78, 156], "Blau"),
        0x08 => ([17, 17, 17], "Schwarz"),
        0x0A => ([184, 145, 47], "Gold"),
        0x02 => ([51, 51, 51], "Sonderfarbe"),
        _ => return None,
    })
}

/// The look of the loaded media; `colors` = (tape colour id, text colour id) from a PT status.
pub fn look(family: Family, colors: Option<(u8, u8)>) -> Look {
    match (family, colors) {
        (Family::Pt, Some((t, i))) => {
            let (tape, tape_name) = tape(t).unwrap_or((PAPER.tape, "Unbekannt"));
            let (ink, ink_name) = ink(i).unwrap_or((PAPER.ink, "Unbekannt"));
            Look { tape, ink, tape_name, ink_name }
        }
        _ => PAPER,
    }
}
