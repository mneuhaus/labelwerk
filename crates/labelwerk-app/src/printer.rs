//! Printer status polling and printing, run on background threads.

use std::sync::Mutex;

use anyhow::{Result, anyhow};
use labelwerk_core::transport::{self, SystemQueue, UsbPrinter};
use labelwerk_core::{Bitmap, Media, Model, PrintOptions};

/// One USB conversation at a time (polling and printing both claim the interface).
static USB: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq)]
pub enum PrinterState {
    Searching,
    /// No printer on USB; a system queue may still exist.
    Missing { queue: Option<SystemQueue> },
    Ready { model: &'static Model, media: Option<&'static Media> },
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
    match UsbPrinter::open(device).and_then(|mut p| p.request_status()) {
        Ok(s) => {
            let media = s.media();
            let errors = s.errors();
            match s.model() {
                None => PrinterState::Problem {
                    name: device.product.clone(),
                    model: None,
                    media: None,
                    message: "Modell unbekannt".into(),
                },
                Some(model) if !errors.is_empty() => PrinterState::Problem {
                    name: model.name.clone(),
                    model: Some(model),
                    media,
                    message: errors.iter().map(|e| german(e)).collect::<Vec<_>>().join(", "),
                },
                Some(model) if media.is_none() => PrinterState::Problem {
                    name: model.name.clone(),
                    model: Some(model),
                    media,
                    message: "Unbekanntes Band / Etikett eingelegt".into(),
                },
                Some(model) => PrinterState::Ready { model, media },
            }
        }
        Err(e) => PrinterState::Busy { product: device.product.clone(), message: format!("{e:#}"), queue: queue() },
    }
}

/// Print over USB, checking printer and media first. Blocking.
pub fn print_usb(model: &'static Model, media: &'static Media, pages: Vec<Bitmap>, opts: PrintOptions) -> Result<()> {
    let _guard = USB.lock().map_err(|_| anyhow!("USB lock poisoned"))?;
    let devices = transport::list_usb()?;
    let device = devices.first().ok_or_else(|| anyhow!("Kein Drucker an USB gefunden"))?;
    let mut printer = UsbPrinter::open(device)?;
    let refs: Vec<&Bitmap> = pages.iter().collect();
    transport::print_usb(&mut printer, model, media, &refs, &opts, |_| {}).map_err(|e| anyhow!(german(&format!("{e:#}"))))
}

pub fn print_queue(
    queue: SystemQueue,
    model: &'static Model,
    media: &'static Media,
    pages: Vec<Bitmap>,
    opts: PrintOptions,
) -> Result<()> {
    let refs: Vec<&Bitmap> = pages.iter().collect();
    transport::print_system_queue(&queue, model, media, &refs, &opts)
}

/// The core speaks English; the app shows the printer's messages in German.
pub fn german(msg: &str) -> String {
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
