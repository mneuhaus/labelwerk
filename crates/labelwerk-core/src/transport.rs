//! Getting bytes to the printer: direct USB (with status) or a system print queue (raw, no status).
//!
//! P-touch Editor talks to the printer over USB directly, so does `UsbPrinter`: through `nusb` on macOS
//! and Linux, through the usbprint.sys device interface on Windows (see `unix` and `win`).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::bitmap::Bitmap;
use crate::media::Media;
use crate::model::{Model, Support};
use crate::protocol::{INITIALIZE, PrintOptions, STATUS_REQUEST, encode_job};
use crate::status::{STATUS_LEN, Status, StatusType};

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
use unix as sys;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as sys;

pub const BROTHER_VID: u16 = 0x04F9;
/// Zero bytes before a status request: enough to flush half a raster line of any model.
const STATUS_INVALIDATE: usize = 400;

/// `LABELWERK_DEBUG=1` traces the USB conversation on stderr.
fn trace(msg: impl FnOnce() -> String) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    if std::env::var_os("LABELWERK_DEBUG").is_some() {
        let t = START.get_or_init(Instant::now).elapsed();
        eprintln!("[usb {:6.3}s] {}", t.as_secs_f32(), msg());
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UsbDevice {
    pub product: String,
    pub serial: Option<String>,
    pub product_id: u16,
    #[serde(skip)]
    info: sys::DeviceRef,
}

/// Brother label printers (QL, PT) on USB.
pub fn list_usb() -> Result<Vec<UsbDevice>> {
    trace(|| "list devices".into());
    let mut found = sys::list()?;
    // macOS can list one device twice for a moment (after another process let go of it)
    let mut seen = std::collections::HashSet::new();
    found.retain(|d| d.serial.is_none() || seen.insert((d.product_id, d.serial.clone())));
    trace(|| format!("found {} Brother printer(s)", found.len()));
    Ok(found)
}

pub struct UsbPrinter {
    pub device: UsbDevice,
    reader: sys::Reader,
    writer: sys::Writer,
}

impl UsbPrinter {
    pub fn open(device: &UsbDevice) -> Result<Self> {
        trace(|| format!("open {} ({:04x})", device.product, device.product_id));
        let (reader, writer) = sys::open(device)?;
        Ok(Self { device: device.clone(), reader, writer })
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<()> {
        trace(|| format!("write {} bytes", data.len()));
        self.writer.write_all(data).context("sending to printer")?;
        self.writer.flush().context("sending to printer")?;
        trace(|| "write done".into());
        Ok(())
    }

    /// Next status block the printer sends, or `None` after `timeout`.
    pub fn read_status(&mut self, timeout: Duration) -> Result<Option<Status>> {
        read_status(&mut self.reader, timeout)
    }

    /// Reset the printer's receive state and ask for its status.
    pub fn request_status(&mut self) -> Result<Status> {
        let mut cmd = vec![0u8; STATUS_INVALIDATE];
        cmd.extend_from_slice(&INITIALIZE);
        cmd.extend_from_slice(&STATUS_REQUEST);
        self.write_all(&cmd)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Some(s) = self.read_status(Duration::from_millis(500))?
                && s.status_type == StatusType::Reply
            {
                return Ok(s);
            }
        }
        bail!("{} did not answer the status request", self.device.product)
    }
}

fn read_status(reader: &mut sys::Reader, timeout: Duration) -> Result<Option<Status>> {
    reader.set_read_timeout(timeout);
    let mut buf = [0u8; STATUS_LEN];
    match reader.read_exact(&mut buf) {
        Ok(()) => {
            trace(|| format!("status {:02x?}", buf));
            Status::parse(&buf).map(Some)
        }
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
        Err(e) => Err(e).context("reading printer status"),
    }
}

/// What we can learn about a connected printer.
#[derive(Debug, Clone)]
pub enum Probe {
    Status(Status),
    /// A model that does not answer status requests (PT-E550W, PT-P750W), known from its USB name.
    Silent(&'static Model),
}

impl Probe {
    pub fn model(&self) -> Option<&'static Model> {
        match self {
            Probe::Status(s) => s.model(),
            Probe::Silent(m) => Some(m),
        }
    }
}

pub fn probe(printer: &mut UsbPrinter) -> Result<Probe> {
    if let Some(model) = Model::by_product(&printer.device.product).filter(|m| !m.protocol.status) {
        return Ok(Probe::Silent(model));
    }
    printer.request_status().map(Probe::Status)
}

/// What happens during a print, for progress display.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintEvent {
    Sending,
    Printing { done: usize, total: usize },
    Cooling,
    Finished,
    /// Sent to a printer that cannot confirm what it printed.
    Unconfirmed,
}

/// Check the printer and media, send the job and wait until every label is out.
///
/// The printer's status messages are read on a second thread while the job is sent: some models report
/// right after the raster-mode command and stop accepting data until that report is collected.
pub fn print_usb(
    printer: &mut UsbPrinter,
    model: &Model,
    media: &Media,
    pages: &[&Bitmap],
    opts: &PrintOptions,
    mut on_event: impl FnMut(PrintEvent),
) -> Result<()> {
    if model.protocol.support == Support::Unsupported {
        bail!("{} speaks a protocol Labelwerk does not implement", model.name);
    }
    if model.protocol.status {
        let status = printer.request_status()?;
        check_ready(&status, model, media)?;
    }
    let job = encode_job(model, media, pages, opts)?;
    let total = pages.len();
    let (tx, rx) = mpsc::channel::<Result<Status>>();
    let stop = AtomicBool::new(false);
    let UsbPrinter { reader, writer, device } = printer;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                match read_status(reader, Duration::from_millis(200)) {
                    Ok(Some(s)) => {
                        if tx.send(Ok(s)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        let result = (|| {
            on_event(PrintEvent::Sending);
            trace(|| format!("write job, {} bytes", job.len()));
            writer.write_all(&job).context("sending to printer")?;
            writer.flush().context("sending to printer")?;
            trace(|| "job sent".into());
            if !model.protocol.status {
                // Nothing will confirm the print; only report an error the printer volunteers.
                if let Ok(Ok(s)) = rx.recv_timeout(Duration::from_secs(2))
                    && s.has_error()
                {
                    bail!("{}", describe_errors(&s));
                }
                on_event(PrintEvent::Unconfirmed);
                return Ok(());
            }
            let mut done = 0;
            on_event(PrintEvent::Printing { done, total });
            let mut last = Instant::now();
            while done < total {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(Ok(s)) if s.has_error() => bail!("{}", describe_errors(&s)),
                    Ok(Ok(s)) if s.status_type == StatusType::PrintingCompleted => {
                        done += 1;
                        last = Instant::now();
                        on_event(PrintEvent::Printing { done, total });
                    }
                    Ok(Ok(s)) if s.cooling() => {
                        last = Instant::now();
                        on_event(PrintEvent::Cooling);
                    }
                    Ok(Ok(_)) => last = Instant::now(),
                    Ok(Err(e)) => return Err(e),
                    Err(mpsc::RecvTimeoutError::Timeout) if last.elapsed() > Duration::from_secs(60) => {
                        bail!("the printer stopped answering ({done} of {total} labels printed)")
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => bail!("lost the connection to {}", device.product),
                }
            }
            on_event(PrintEvent::Finished);
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        result
    })
}

pub fn check_ready(status: &Status, model: &Model, media: &Media) -> Result<()> {
    match status.model() {
        None => bail!("{} is not a known QL or PT model", status.model_name()),
        Some(m) if m.name != model.name => bail!("{} is connected, but this label is for a {}", m.name, model.name),
        Some(_) => {}
    }
    if status.has_error() {
        bail!("{}", describe_errors(status));
    }
    match status.media() {
        Some(loaded) if loaded.id == media.id => Ok(()),
        Some(loaded) => bail!("{} is loaded, but this label is for {}", loaded.label(), media.label()),
        None => bail!("The printer reports media it does not know ({} mm, type {:#04x})", status.media_width_mm, status.media_type),
    }
}

fn describe_errors(status: &Status) -> String {
    let errors = status.errors();
    if errors.is_empty() { "The printer reported an error".into() } else { errors.join(", ") }
}

/// A system print queue for a supported Brother printer: CUPS on macOS and Linux, the spooler on Windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemQueue {
    pub name: String,
    /// CUPS device URI, or the Windows port name
    pub uri: String,
}

pub fn list_system_queues() -> Vec<SystemQueue> {
    sys::list_queues()
}

/// Hand the raw job to the system queue. No status, no media check: the printer's own error light is
/// the only feedback.
pub fn print_system_queue(
    queue: &SystemQueue,
    model: &Model,
    media: &Media,
    pages: &[&Bitmap],
    opts: &PrintOptions,
) -> Result<()> {
    sys::send_to_queue(queue, &encode_job(model, media, pages, opts)?)
}
