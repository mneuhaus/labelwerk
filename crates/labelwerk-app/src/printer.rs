//! Printer status polling and printing, run on background threads.

use std::sync::Mutex;

use anyhow::{Result, anyhow};
use labelwerk_core::model::Support;
use labelwerk_core::transport::{self, PrintEvent, Probe, SystemQueue, UsbPrinter};
use labelwerk_core::{Bitmap, Media, Model, PrintOptions};

use crate::{i18n, tr};

/// One USB conversation at a time (polling and printing both claim the interface).
static USB: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq)]
pub enum PrinterState {
    Searching,
    /// No printer on USB; a system queue may still exist.
    Missing { queue: Option<SystemQueue> },
    /// `colors`: (tape, print) colour ids reported by PT printers.
    Ready { model: &'static Model, media: Option<&'static Media>, colors: Option<(u8, u8)> },
    Problem { name: String, model: Option<&'static Model>, media: Option<&'static Media>, message: String },
    /// Found on USB but could not be opened (another program is using it).
    Busy { product: String, message: String, queue: Option<SystemQueue> },
}

impl PrinterState {
    pub fn model(&self) -> Option<&'static Model> {
        match self {
            PrinterState::Ready { model, .. } => Some(model),
            PrinterState::Problem { model, .. } => *model,
            _ => None,
        }
    }

    pub fn colors(&self) -> Option<(u8, u8)> {
        match self {
            PrinterState::Ready { colors, .. } => *colors,
            _ => None,
        }
    }

    pub fn loaded_media(&self) -> Option<&'static Media> {
        match self {
            PrinterState::Ready { media, .. } | PrinterState::Problem { media, .. } => *media,
            _ => None,
        }
    }
}

/// Look for the printer and ask for its status. Blocking.
pub fn poll() -> PrinterState {
    let Ok(_guard) = USB.try_lock() else {
        return PrinterState::Searching; // printing right now
    };
    let queue = || transport::list_system_queues().into_iter().next();
    let devices = match transport::list_usb() {
        Ok(d) => d,
        Err(_) => return PrinterState::Missing { queue: queue() },
    };
    let Some(device) = devices.first() else {
        return PrinterState::Missing { queue: queue() };
    };
    match UsbPrinter::open(device).and_then(|mut p| transport::probe(&mut p)) {
        Ok(Probe::Silent(model)) => PrinterState::Ready { model, media: None, colors: None },
        Ok(Probe::Status(s)) => {
            let media = s.media();
            let errors = s.errors();
            match s.model() {
                None => PrinterState::Problem {
                    name: device.product.clone(),
                    model: None,
                    media: None,
                    message: tr!("Unknown model", "Modell unbekannt").to_string(),
                },
                Some(model) if !errors.is_empty() => PrinterState::Problem {
                    name: model.name.clone(),
                    model: Some(model),
                    media,
                    message: errors.iter().map(|e| localize(e)).collect::<Vec<_>>().join(", "),
                },
                Some(model) if model.protocol.support == Support::Unsupported => PrinterState::Problem {
                    name: model.name.clone(),
                    model: Some(model),
                    media,
                    message: tr!("Model is not supported (different protocol)", "Modell wird nicht unterstützt (anderes Protokoll)").to_string(),
                },
                Some(model) if media.is_none() => PrinterState::Problem {
                    name: model.name.clone(),
                    model: Some(model),
                    media,
                    message: tr!("Unknown tape / label loaded", "Unbekanntes Band / Etikett eingelegt").to_string(),
                },
                Some(model) => {
                    let colors = (model.family == labelwerk_core::Family::Pt).then_some((s.tape_color, s.text_color));
                    PrinterState::Ready { model, media, colors }
                }
            }
        }
        Err(e) => PrinterState::Busy { product: device.product.clone(), message: format!("{e:#}"), queue: queue() },
    }
}

/// Print over USB, checking printer and media first. Blocking. `Ok(false)`: sent, but the model cannot confirm.
pub fn print_usb(model: &'static Model, media: &'static Media, pages: Vec<Bitmap>, opts: PrintOptions) -> Result<bool> {
    let _guard = USB.lock().map_err(|_| anyhow!(tr!("USB lock poisoned", "USB-Sperre beschädigt")))?;
    let devices = transport::list_usb()?;
    let device = devices.first().ok_or_else(|| anyhow!(tr!("No printer found on USB", "Kein Drucker an USB gefunden")))?;
    let mut printer = UsbPrinter::open(device)?;
    let refs: Vec<&Bitmap> = pages.iter().collect();
    let mut confirmed = true;
    transport::print_usb(&mut printer, model, media, &refs, &opts, |event| {
        if event == PrintEvent::Unconfirmed {
            confirmed = false;
        }
    })
    .map_err(|e| anyhow!(localize(&format!("{e:#}"))))?;
    Ok(confirmed)
}

pub fn print_queue(
    queue: SystemQueue,
    model: &'static Model,
    media: &'static Media,
    pages: Vec<Bitmap>,
    opts: PrintOptions,
) -> Result<bool> {
    let refs: Vec<&Bitmap> = pages.iter().collect();
    transport::print_system_queue(&queue, model, media, &refs, &opts)?;
    Ok(false)
}

/// The core speaks English; map its messages to German when the UI is German, else leave them as is.
pub fn localize(msg: &str) -> String {
    if !i18n::german() {
        return msg.to_string();
    }
    const MAP: &[(&str, &str)] = &[
        ("No media loaded", "Kein Band / keine Etiketten eingelegt"),
        ("Cutter jam", "Schneidmesser blockiert"),
        ("Printer turned off", "Drucker ist aus"),
        ("Wrong media loaded for this label", "Falsches Band für dieses Etikett"),
        ("Expansion buffer full", "Druckerspeicher voll"),
        ("Communication error", "Übertragungsfehler"),
        ("Cover open", "Deckel offen"),
        ("Media cannot be fed (end of roll?)", "Rolle lässt sich nicht einziehen (leer?)"),
        ("System error", "Systemfehler im Drucker"),
        ("Weak batteries", "Akku schwach"),
        ("High-voltage adapter", "Falsches Netzteil"),
        ("Overheating", "Überhitzt, kurz abkühlen lassen"),
        ("did not answer the status request", "antwortet nicht"),
        ("the printer stopped answering", "Der Drucker antwortet nicht mehr"),
        ("is loaded, but this label is for", "ist eingelegt, das Etikett ist aber für"),
        ("is connected, but this label is for a", "ist angeschlossen, das Etikett ist aber für"),
    ];
    let mut out = msg.to_string();
    for (en, de) in MAP {
        out = out.replace(en, de);
    }
    out.replace("mm endless", "mm Endlos").replace("mm tape", "mm Band")
}
