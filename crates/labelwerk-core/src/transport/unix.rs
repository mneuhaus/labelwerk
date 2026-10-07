//! macOS and Linux: USB through `nusb` (bulk endpoints, as P-touch Editor does over IOKit), system queues
//! through CUPS.

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nusb::MaybeFuture;
use nusb::transfer::{Bulk, In, Out};

use super::{BROTHER_VID, SystemQueue, UsbDevice, trace};

const PRINTER_CLASS: u8 = 0x07;

pub type DeviceRef = nusb::DeviceInfo;
pub type Reader = nusb::io::EndpointRead<Bulk>;
pub type Writer = nusb::io::EndpointWrite<Bulk>;

pub fn list() -> Result<Vec<UsbDevice>> {
    let devices = nusb::list_devices().wait().context("listing USB devices")?;
    Ok(devices
        .filter(|d| d.vendor_id() == BROTHER_VID)
        .filter(|d| {
            d.product_string().is_some_and(|p| p.contains("QL-") || p.contains("PT-"))
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

pub fn open(device: &UsbDevice) -> Result<(Reader, Writer)> {
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
    trace(|| format!("claimed interface {number}, in {ep_in:#04x}, out {ep_out:#04x}"));
    let reader = intf.endpoint::<Bulk, In>(ep_in)?.reader(64);
    let writer = intf.endpoint::<Bulk, Out>(ep_out)?.writer(16 * 1024).with_write_timeout(Duration::from_secs(30));
    Ok((reader, writer))
}

pub fn list_queues() -> Vec<SystemQueue> {
    let Ok(out) = std::process::Command::new("lpstat").arg("-v").env("LANG", "C").env("LC_ALL", "C").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (head, uri) = line.split_once(": ")?;
            let name = head.split_whitespace().last()?.to_string();
            let wanted = ["QL-", "QL_", "PT-", "PT_"];
            wanted.iter().any(|w| uri.contains(w) || name.contains(w)).then(|| SystemQueue { name, uri: uri.trim().to_string() })
        })
        .collect()
}

pub fn send_to_queue(queue: &SystemQueue, job: &[u8]) -> Result<()> {
    let mut child = std::process::Command::new("lp")
        .args(["-d", &queue.name, "-o", "raw", "-t", "Labelwerk"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("starting lp")?;
    child.stdin.take().expect("piped stdin").write_all(job)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("lp failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
