//! The 32-byte status block the printer sends after `ESC i S` and on its own while printing.

use anyhow::{Result, ensure};
use serde::Serialize;

use crate::media::Media;
use crate::model::{Family, Model};

pub const STATUS_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StatusType {
    Reply,
    PrintingCompleted,
    Error,
    TurnedOff,
    Notification,
    PhaseChange,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    Receiving,
    Printing,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub series_code: u8,
    pub model_code: u8,
    pub error1: u8,
    pub error2: u8,
    pub media_width_mm: u8,
    pub media_type: u8,
    pub media_length_mm: u8,
    pub mode: u8,
    pub status_type: StatusType,
    pub phase: Phase,
    pub notification: u8,
}

/// (byte, mask, message) for the error flags of the QL series.
const QL_ERRORS: &[(u8, u8, &str)] = &[
    (1, 0x01, "No media loaded"),
    (1, 0x04, "Cutter jam"),
    (1, 0x20, "Printer turned off"),
    (2, 0x01, "Wrong media loaded for this label"),
    (2, 0x02, "Expansion buffer full"),
    (2, 0x04, "Communication error"),
    (2, 0x10, "Cover open"),
    (2, 0x40, "Media cannot be fed (end of roll?)"),
    (2, 0x80, "System error"),
];

/// The same for the PT series (Raster Command Reference PT-E550W/P750W/P710BT, status tables 1 and 2).
const PT_ERRORS: &[(u8, u8, &str)] = &[
    (1, 0x01, "No media loaded"),
    (1, 0x04, "Cutter jam"),
    (1, 0x08, "Weak batteries"),
    (1, 0x40, "High-voltage adapter"),
    (2, 0x01, "Wrong media loaded for this label"),
    (2, 0x10, "Cover open"),
    (2, 0x20, "Overheating"),
];

impl Status {
    pub fn parse(b: &[u8]) -> Result<Self> {
        ensure!(b.len() >= STATUS_LEN, "status too short: {} bytes", b.len());
        ensure!(b[0] == 0x80 && b[1] == 0x20, "not a status block: {:02x?}", &b[..4]);
        Ok(Self {
            series_code: b[3],
            model_code: b[4],
            error1: b[8],
            error2: b[9],
            media_width_mm: b[10],
            media_type: b[11],
            mode: b[15],
            media_length_mm: b[17],
            status_type: match b[18] {
                0x00 => StatusType::Reply,
                0x01 => StatusType::PrintingCompleted,
                0x02 => StatusType::Error,
                0x04 => StatusType::TurnedOff,
                0x05 => StatusType::Notification,
                0x06 => StatusType::PhaseChange,
                x => StatusType::Other(x),
            },
            phase: match b[19] {
                0x00 => Phase::Receiving,
                0x01 => Phase::Printing,
                x => Phase::Other(x),
            },
            notification: b[22],
        })
    }

    pub fn model(&self) -> Option<&'static Model> {
        Model::by_codes(self.series_code, self.model_code)
    }

    pub fn model_name(&self) -> String {
        match self.model() {
            Some(m) => m.name.clone(),
            None => format!("Brother printer {:02X}/{:02X}", self.series_code, self.model_code),
        }
    }

    pub fn is_supported_model(&self) -> bool {
        self.model().is_some()
    }

    pub fn errors(&self) -> Vec<&'static str> {
        let table = match self.model().map(|m| m.family) {
            Some(Family::Pt) => PT_ERRORS,
            _ => QL_ERRORS,
        };
        table
            .iter()
            .filter(|(byte, mask, _)| (if *byte == 1 { self.error1 } else { self.error2 }) & mask != 0)
            .map(|(_, _, msg)| *msg)
            .collect()
    }

    pub fn has_error(&self) -> bool {
        self.error1 != 0 || self.error2 != 0 || self.status_type == StatusType::Error
    }

    /// The loaded media, if the printer reports one we know.
    pub fn media(&self) -> Option<&'static Media> {
        self.model()?.media_from_status(self.media_type, self.media_width_mm, self.media_length_mm)
    }

    pub fn cooling(&self) -> bool {
        self.status_type == StatusType::Notification && self.notification == 0x03
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(f: impl FnOnce(&mut [u8; 32])) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&[0x80, 0x20, b'B', b'4', b'C', b'0', 0, 0]);
        f(&mut b);
        b
    }

    #[test]
    fn idle_printer_with_62mm_tape() {
        let s = Status::parse(&block(|b| {
            b[10] = 62;
            b[11] = 0x0A;
        }))
        .unwrap();
        assert_eq!(s.model_name(), "QL-1100");
        assert_eq!(s.status_type, StatusType::Reply);
        assert!(!s.has_error());
        assert_eq!(s.media().unwrap().key(), "62");
    }

    #[test]
    fn cover_open_and_no_media() {
        let s = Status::parse(&block(|b| {
            b[8] = 0x01;
            b[9] = 0x10;
            b[18] = 0x02;
        }))
        .unwrap();
        assert_eq!(s.errors(), ["No media loaded", "Cover open"]);
        assert!(s.media().is_none());
    }

    /// Captured from a PT-P710BT with 24 mm white laminated tape (2026-10-06).
    #[test]
    fn real_p710bt_reply() {
        let raw = [
            0x80, 0x20, 0x42, 0x30, 0x76, 0x30, 0x00, 0x00, 0x00, 0x00, 0x18, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let s = Status::parse(&raw).unwrap();
        assert_eq!(s.model_name(), "PT-P710BT");
        assert!(!s.has_error());
        assert_eq!(s.media().unwrap().key(), "24");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Status::parse(&[0u8; 32]).is_err());
        assert!(Status::parse(&[0x80, 0x20]).is_err());
    }
}
