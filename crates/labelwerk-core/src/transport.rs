//! Getting bytes to the printer: direct USB (with status) or a system print queue (raw, no status).
//!
//! P-touch Editor talks to the printer over USB directly (IOKit bulk endpoints), so does `UsbPrinter`
//! via `nusb`. On Windows `nusb` needs the WinUSB driver; there the spooler route is the way to go.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use nusb::MaybeFuture;
use nusb::transfer::{Bulk, In, Out};
use serde::Serialize;

use crate::bitmap::Bitmap;
use crate::media::Media;
use crate::protocol::{INITIALIZE, INVALIDATE_BYTES, PrintOptions, STATUS_REQUEST, encode_job};
use crate::status::{STATUS_LEN, Status, StatusType};

pub const BROTHER_VID: u16 = 0x04F9;
const PRINTER_CLASS: u8 = 0x07;

#[derive(Debug, Clone, Serialize)]
pub struct UsbDevice {
    pub product: String,
    pub serial: Option<String>,
    pub product_id: u16,
    #[serde(skip)]
    info: nusb::DeviceInfo,
}

/// Brother label printers on USB.
pub fn list_usb() -> Result<Vec<UsbDevice>> {
    let devices = nusb::list_devices().wait().context("listing USB devices")?;
    Ok(devices
        .filter(|d| d.vendor_id() == BROTHER_VID)
        .filter(|d| {
            d.product_string().is_some_and(|p| p.contains("QL-"))
                || d.interfaces().any(|i| i.class() == PRINTER_CLASS)
        })
        .map(|d| UsbDevice {
            product: d.product_string().unwrap_or("Brother printer").to_string(),
            serial: d.serial_number().map(str::to_string),
            product_id: d.product_id(),
            info: d,
        })
        .collect())
}

pub struct UsbPrinter {
    pub device: UsbDevice,
    reader: nusb::io::EndpointRead<Bulk>,
    writer: nusb::io::EndpointWrite<Bulk>,
}

impl UsbPrinter {
    pub fn open(device: &UsbDevice) -> Result<Self> {
        let dev = device.info.open().wait().with_context(|| format!("opening {}", device.product))?;
        let config = dev.active_configuration().context("reading USB configuration")?;
        let (number, ep_in, ep_out) = config
            .interfaces()
            .filter_map(|i| i.alt_settings().next())
            .find_map(|alt| {
                let bulk = |dir| {
                    alt.endpoints()
                        .find(|e| e.transfer_type() == nusb::descriptors::TransferType::Bulk && e.direction() == dir)
                        .map(|e| e.address())
                };
                Some((alt.interface_number(), bulk(nusb::transfer::Direction::In)?, bulk(nusb::transfer::Direction::Out)?))
            })
            .ok_or_else(|| anyhow!("{} has no bulk printer interface", device.product))?;
        let intf = dev
            .detach_and_claim_interface(number)
            .wait()
            .with_context(|| format!("claiming {} (is another program printing to it?)", device.product))?;
        let reader = intf.endpoint::<Bulk, In>(ep_in)?.reader(64);
        let writer = intf.endpoint::<Bulk, Out>(ep_out)?.writer(16 * 1024).with_write_timeout(Duration::from_secs(30));
        Ok(Self { device: device.clone(), reader, writer })
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data).context("sending to printer")?;
        self.writer.flush().context("sending to printer")?;
        Ok(())
    }

    /// Next status block the printer sends, or `None` after `timeout`.
    pub fn read_status(&mut self, timeout: Duration) -> Result<Option<Status>> {
        self.reader.set_read_timeout(timeout);
        let mut buf = [0u8; STATUS_LEN];
        match self.reader.read_exact(&mut buf) {
            Ok(()) => Status::parse(&buf).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
            Err(e) => Err(e).context("reading printer status"),
        }
    }

    /// Reset the printer's receive state and ask for its status.
    pub fn request_status(&mut self) -> Result<Status> {
        let mut cmd = vec![0u8; INVALIDATE_BYTES];
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

/// What happens during a print, for progress display.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintEvent {
    Sending,
    Printing { done: usize, total: usize },
    Cooling,
    Finished,
}

/// Check the printer and media, send the job and wait until every label is out.
pub fn print_usb(
    printer: &mut UsbPrinter,
    media: &Media,
    pages: &[&Bitmap],
    opts: &PrintOptions,
    mut on_event: impl FnMut(PrintEvent),
) -> Result<()> {
    let status = printer.request_status()?;
    check_ready(&status, media)?;
    let job = encode_job(media, pages, opts)?;
    on_event(PrintEvent::Sending);
    printer.write_all(&job)?;
    let total = pages.len();
    let mut done = 0;
    on_event(PrintEvent::Printing { done, total });
    let mut last = Instant::now();
    while done < total {
        match printer.read_status(Duration::from_secs(1))? {
            Some(s) if s.has_error() => bail!("{}", describe_errors(&s)),
            Some(s) if s.status_type == StatusType::PrintingCompleted => {
                done += 1;
                last = Instant::now();
                on_event(PrintEvent::Printing { done, total });
            }
            Some(s) if s.cooling() => {
                last = Instant::now();
                on_event(PrintEvent::Cooling);
            }
            Some(_) => last = Instant::now(),
            None if last.elapsed() > Duration::from_secs(60) => {
                bail!("the printer stopped answering ({done} of {total} labels printed)")
            }
            None => {}
        }
    }
    on_event(PrintEvent::Finished);
    Ok(())
}

pub fn check_ready(status: &Status, media: &Media) -> Result<()> {
    if !status.is_supported_model() {
        bail!("{} is not supported (QL-1100, QL-1110NWB and QL-1115NWB are)", status.model_name());
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

/// A CUPS queue that points at a supported Brother printer (macOS, Linux).
#[derive(Debug, Clone, Serialize)]
pub struct SystemQueue {
    pub name: String,
    pub uri: String,
}

pub fn list_system_queues() -> Vec<SystemQueue> {
    let Ok(out) = std::process::Command::new("lpstat").arg("-v").env("LANG", "C").env("LC_ALL", "C").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (head, uri) = line.split_once(": ")?;
            let name = head.split_whitespace().last()?.to_string();
            let wanted = ["QL-1100", "QL-1110", "QL-1115", "QL_1100", "QL_1110", "QL_1115"];
            wanted.iter().any(|w| uri.contains(w) || name.contains(w)).then(|| SystemQueue { name, uri: uri.trim().to_string() })
        })
        .collect()
}

/// Hand the raw job to the system queue. No status, no media check: the printer's own error light is
/// the only feedback.
pub fn print_system_queue(queue: &SystemQueue, media: &Media, pages: &[&Bitmap], opts: &PrintOptions) -> Result<()> {
    let job = encode_job(media, pages, opts)?;
    let mut child = std::process::Command::new("lp")
        .args(["-d", &queue.name, "-o", "raw", "-t", "Labelwerk"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("starting lp")?;
    child.stdin.take().expect("piped stdin").write_all(&job)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("lp failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
