//! Remembered state: the label being edited, settings and recently printed labels.

use std::path::PathBuf;

use labelwerk_core::Label;
use serde::{Deserialize, Serialize};

pub const HISTORY_LEN: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub label: Label,
    /// Printer model the label is designed for ("QL-1100", "PT-P710BT").
    pub model: String,
    /// Media key ("62", "62x29", ...).
    pub media: String,
    /// Switch to the roll the printer reports whenever it changes.
    pub follow_printer: bool,
    pub copies: u32,
    pub history: Vec<HistoryEntry>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            label: Label::default(),
            model: "QL-1100".into(),
            media: "62".into(),
            follow_printer: true,
            copies: 1,
            history: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub label: Label,
    #[serde(default)]
    pub model: String,
    pub media: String,
}

impl HistoryEntry {
    pub fn title(&self) -> String {
        let first = self.label.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        if first.is_empty() { "(nur QR-Code)".into() } else { first.to_string() }
    }
}

fn path() -> Option<PathBuf> {
    // LABELWERK_STATE: separate state for tests and screenshots
    if let Ok(p) = std::env::var("LABELWERK_STATE") {
        return Some(PathBuf::from(p));
    }
    Some(dirs::data_dir()?.join("Labelwerk").join("state.json"))
}

pub fn load() -> Saved {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(saved: &Saved) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec_pretty(saved) {
        Ok(bytes) => {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &p)).is_err() {
                eprintln!("could not save {}", p.display());
            }
        }
        Err(e) => eprintln!("could not save state: {e}"),
    }
}

/// Put a printed label on top of the history, without duplicates.
pub fn remember(history: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    history.retain(|e| e != &entry);
    history.insert(0, entry);
    history.truncate(HISTORY_LEN);
}
