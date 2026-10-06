//! The 32-byte status block the printer sends after `ESC i S` and on its own while printing.

use anyhow::{Result, ensure};
use serde::Serialize;

use crate::media::Media;

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

/// (byte, mask, message) for every error flag the QL-1100 uses.
const ERRORS: &[(u8, u8, &str)] = &[
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

    pub fn model_name(&self) -> &'static str {
        match (self.series_code, self.model_code) {
            (0x34, 0x43) => "QL-1100",
            (0x34, 0x44) => "QL-1110NWB",
            (0x34, 0x45) => "QL-1115NWB",
            _ => "unknown Brother printer",
        }
    }

    pub fn is_supported_model(&self) -> bool {
        self.series_code == 0x34 && matches!(self.model_code, 0x43..=0x45)
    }

    pub fn errors(&self) -> Vec<&'static str> {
        ERRORS
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
        Media::from_status(self.media_type, self.media_width_mm, self.media_length_mm)
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

    #[test]
    fn rejects_garbage() {
        assert!(Status::parse(&[0u8; 32]).is_err());
        assert!(Status::parse(&[0x80, 0x20]).is_err());
    }
}
