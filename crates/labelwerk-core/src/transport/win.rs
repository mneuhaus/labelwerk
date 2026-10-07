//! Windows: a USB printer gets Windows' own "USB Printing Support" driver (usbprint.sys), whose device
//! interface opens like a file for raw two-way I/O. No WinUSB and no Brother driver needed. System queues
//! go through the spooler with the RAW datatype.

use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DOC_INFO_1W, EndDocPrinter, EnumPrintersW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS,
    PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_2W, StartDocPrinterW, WritePrinter,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::core::GUID;

use super::{BROTHER_VID, SystemQueue, UsbDevice, trace};

/// Device interface class of usbprint.sys.
const GUID_DEVINTERFACE_USBPRINT: GUID = GUID::from_u128(0x28d78fad_5a12_11d1_ae5b_0000f803a8c2);
/// CTL_CODE(FILE_DEVICE_UNKNOWN, 13, METHOD_BUFFERED, FILE_ANY_ACCESS): the printer's IEEE 1284 device ID.
const IOCTL_USBPRINT_GET_1284_ID: u32 = 0x0022_0034;
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Path of the usbprint device interface (`\\?\usb#vid_04f9&pid_20af#<serial>#{28d78fad-…}`).
pub type DeviceRef = String;

pub fn list() -> Result<Vec<UsbDevice>> {
    let vid = format!("vid_{BROTHER_VID:04x}");
    let mut found = Vec::new();
    for path in interface_paths()? {
        let mut parts = path.split('#').skip(1);
        let (Some(ids), serial) = (parts.next(), parts.next()) else { continue };
        let ids = ids.to_ascii_lowercase();
        if !ids.split('&').any(|p| p == vid) {
            continue;
        }
        let product_id = ids
            .split('&')
            .find_map(|p| p.strip_prefix("pid_"))
            .and_then(|p| u16::from_str_radix(p, 16).ok())
            .unwrap_or(0);
        let model = device_id_1284(&path).and_then(|id| model_of(&id));
        trace(|| format!("{path}: model {model:?}"));
        // other Brother printers (lasers, inkjets) share the driver; an unreadable ID is worth a try
        if model.as_ref().is_some_and(|m| !m.contains("QL-") && !m.contains("PT-")) {
            continue;
        }
        found.push(UsbDevice {
            product: model.unwrap_or_else(|| "Brother printer".into()),
            // a real serial number, not an instance id Windows made up (those contain '&')
            serial: serial.filter(|s| !s.contains('&')).map(str::to_ascii_uppercase),
            product_id,
            info: path,
        });
    }
    Ok(found)
}

pub fn open(device: &UsbDevice) -> Result<(Reader, Writer)> {
    let file = open_handle(&device.info, FILE_FLAG_OVERLAPPED)
        .with_context(|| format!("opening {} (is another program printing to it?)", device.product))?;
    let file = Arc::new(file);
    let reader = Reader { file: file.clone(), event: event()?, timeout: Duration::from_millis(500) };
    let writer = Writer { file, event: event()? };
    Ok((reader, writer))
}

pub struct Reader {
    file: Arc<OwnedHandle>,
    event: OwnedHandle,
    timeout: Duration,
}

impl Reader {
    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let deadline = Instant::now() + self.timeout;
        let len = buf.len().min(u32::MAX as usize) as u32;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let read = overlapped(&self.file, &self.event, left, |h, ov| unsafe {
                ReadFile(h, buf.as_mut_ptr(), len, null_mut(), ov)
            })?;
            match read {
                Some(n) if n > 0 => return Ok(n),
                // usbprint may complete a read with nothing when the printer has nothing to say
                Some(_) if !left.is_zero() => std::thread::sleep(Duration::from_millis(20).min(left)),
                _ => return Err(io::ErrorKind::TimedOut.into()),
            }
        }
    }
}

pub struct Writer {
    file: Arc<OwnedHandle>,
    event: OwnedHandle,
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(64 * 1024) as u32;
        let written = overlapped(&self.file, &self.event, WRITE_TIMEOUT, |h, ov| unsafe {
            WriteFile(h, buf.as_ptr(), len, null_mut(), ov)
        })?;
        written.ok_or_else(|| io::ErrorKind::TimedOut.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One overlapped read or write, so reading and writing can run on two threads over the same handle.
/// `None` when it did not complete within `timeout`.
fn overlapped(
    file: &OwnedHandle,
    event: &OwnedHandle,
    timeout: Duration,
    start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32,
) -> io::Result<Option<usize>> {
    let h = file.as_raw_handle();
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    ov.hEvent = event.as_raw_handle();
    if start(h, &mut ov) == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_IO_PENDING {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
    let ms = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
    let waited = unsafe { WaitForSingleObject(ov.hEvent, ms) };
    let mut n = 0u32;
    if waited == WAIT_TIMEOUT {
        // `ov` must outlive the operation: cancel, then wait for it to end (or for data that won the race)
        unsafe { CancelIoEx(h, &ov) };
        let done = unsafe { GetOverlappedResult(h, &ov, &mut n, 1) };
        return Ok((done != 0 && n > 0).then_some(n as usize));
    }
    if waited != WAIT_OBJECT_0 || unsafe { GetOverlappedResult(h, &ov, &mut n, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Some(n as usize))
}

fn open_handle(path: &str, flags: FILE_FLAGS_AND_ATTRIBUTES) -> io::Result<OwnedHandle> {
    let path = wide(path);
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

fn event() -> io::Result<OwnedHandle> {
    let h = unsafe { CreateEventW(null(), 1, 0, null()) };
    if h.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

/// Paths of every present usbprint device interface.
fn interface_paths() -> Result<Vec<String>> {
    let set = unsafe {
        SetupDiGetClassDevsW(&GUID_DEVINTERFACE_USBPRINT, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
    };
    if set == INVALID_HANDLE_VALUE as isize {
        bail!("listing USB printers: {}", io::Error::last_os_error());
    }
    let mut paths = Vec::new();
    for index in 0u32.. {
        let mut data: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInterfaces(set, null(), &GUID_DEVINTERFACE_USBPRINT, index, &mut data) } == 0 {
            break;
        }
        let mut needed = 0u32;
        unsafe { SetupDiGetDeviceInterfaceDetailW(set, &data, null_mut(), 0, &mut needed, null_mut()) };
        if needed == 0 {
            continue;
        }
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        if unsafe { SetupDiGetDeviceInterfaceDetailW(set, &data, detail, needed, null_mut(), null_mut()) } != 0 {
            paths.push(unsafe { from_wide(std::ptr::addr_of!((*detail).DevicePath).cast()) });
        }
    }
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    Ok(paths)
}

/// "MFG:Brother;CMD:PT-CBP;MDL:PT-P710BT;CLS:PRINTER;…"
fn device_id_1284(path: &str) -> Option<String> {
    let file = open_handle(path, 0).ok()?;
    let mut buf = [0u8; 1024];
    let mut n = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_USBPRINT_GET_1284_ID,
            null(),
            0,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut n,
            null_mut(),
        )
    };
    // the first two bytes are the length
    (ok != 0 && n > 2).then(|| String::from_utf8_lossy(&buf[2..n as usize]).into_owned())
}

fn model_of(device_id: &str) -> Option<String> {
    device_id.split(';').find_map(|field| {
        let (key, value) = field.split_once(':')?;
        matches!(key.trim(), "MDL" | "MODEL").then(|| value.trim().to_string())
    })
}

pub fn list_queues() -> Vec<SystemQueue> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let (mut needed, mut count) = (0u32, 0u32);
    unsafe { EnumPrintersW(flags, null(), 2, null_mut(), 0, &mut needed, &mut count) };
    if needed == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    if unsafe { EnumPrintersW(flags, null(), 2, buf.as_mut_ptr().cast(), needed, &mut needed, &mut count) } == 0 {
        return Vec::new();
    }
    let printers = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<PRINTER_INFO_2W>(), count as usize) };
    printers
        .iter()
        .filter_map(|p| {
            let name = unsafe { from_wide(p.pPrinterName) };
            let driver = unsafe { from_wide(p.pDriverName) };
            let wanted = ["QL-", "PT-"];
            wanted
                .iter()
                .any(|w| name.contains(w) || driver.contains(w))
                .then(|| SystemQueue { name, uri: unsafe { from_wide(p.pPortName) } })
        })
        .collect()
}

pub fn send_to_queue(queue: &SystemQueue, job: &[u8]) -> Result<()> {
    let name = wide(&queue.name);
    let mut printer = PRINTER_HANDLE { Value: null_mut() };
    if unsafe { OpenPrinterW(name.as_ptr(), &mut printer, null()) } == 0 {
        return Err(io::Error::last_os_error()).with_context(|| format!("opening the queue {}", queue.name));
    }
    let result = (|| {
        let mut doc_name = wide("Labelwerk");
        let mut datatype = wide("RAW");
        let doc = DOC_INFO_1W { pDocName: doc_name.as_mut_ptr(), pOutputFile: null_mut(), pDatatype: datatype.as_mut_ptr() };
        if unsafe { StartDocPrinterW(printer, 1, &doc) } == 0 {
            return Err(io::Error::last_os_error()).context("starting the print job");
        }
        let mut sent = 0;
        while sent < job.len() {
            let chunk = (job.len() - sent).min(1 << 20) as u32;
            let mut n = 0u32;
            if unsafe { WritePrinter(printer, job[sent..].as_ptr().cast(), chunk, &mut n) } == 0 || n == 0 {
                let err = io::Error::last_os_error();
                unsafe { EndDocPrinter(printer) };
                return Err(err).context("sending to the queue");
            }
            sent += n as usize;
        }
        if unsafe { EndDocPrinter(printer) } == 0 {
            return Err(io::Error::last_os_error()).context("finishing the print job");
        }
        Ok(())
    })();
    unsafe { ClosePrinter(printer) };
    result
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// # Safety
/// `p` is null or points at a NUL-terminated UTF-16 string.
unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_from_1284_id() {
        let id = "MFG:Brother;CMD:PT-CBP;MDL:PT-P710BT;CLS:PRINTER;CID:Brother PT-P710BT;";
        assert_eq!(model_of(id).as_deref(), Some("PT-P710BT"));
        assert_eq!(model_of("MANUFACTURER:Brother;MODEL:QL-1100;"), Some("QL-1100".into()));
    }
}
