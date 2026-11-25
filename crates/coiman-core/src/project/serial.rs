//! Serial ports, for a board on USB (Arduino, ESP32, a Pico…): the ports there are, opening
//! one with its speed and format, reading what the board prints and sending it text. The
//! Serial Monitor extension shows what comes (through the plugin host's `serial-*` calls).
//!
//! A Windows named pipe (`\\.\pipe\name`) or a Unix FIFO opens too, with no settings to
//! apply: a program can stand in for a board.

use std::io;
use std::time::Duration;

/// How the bytes are framed: data bits, parity, stop bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub two_stop_bits: bool,
    /// Data Terminal Ready: many boards (Arduino) restart when it goes on.
    pub dtr: bool,
    pub rts: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { baud: 115_200, data_bits: 8, parity: Parity::None, two_stop_bits: false, dtr: true, rts: true }
    }
}

impl Settings {
    /// The format as it's usually written: `8N1`, `7E1`.
    pub fn format(&self) -> String {
        let p = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        format!("{}{p}{}", self.data_bits, if self.two_stop_bits { 2 } else { 1 })
    }
}

/// A port there is: what to open, and what it is when that's known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub path: String,
    pub description: String,
}

/// An open port. Reading waits a little (about 50 ms) and gives 0 bytes when nothing came;
/// writing may happen from another thread meanwhile.
pub struct Port {
    inner: imp::Port,
}

impl Port {
    pub fn open(path: &str, settings: &Settings) -> Result<Port, String> {
        imp::Port::open(path, settings).map(|inner| Port { inner })
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }

    pub fn write_all(&self, data: &[u8]) -> io::Result<()> {
        let mut rest = data;
        while !rest.is_empty() {
            let n = self.inner.write(rest)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "the port took nothing"));
            }
            rest = &rest[n..];
        }
        Ok(())
    }

    /// Sets DTR and RTS (a pipe has neither: nothing happens).
    pub fn set_lines(&self, dtr: bool, rts: bool) -> io::Result<()> {
        self.inner.set_lines(dtr, rts)
    }

    /// Whether it's a serial port (settings applied) or something standing in for one.
    pub fn is_serial(&self) -> bool {
        self.inner.serial
    }
}

/// How long a read waits for something to come.
const WAIT: Duration = Duration::from_millis(50);

/// The serial ports on this computer.
pub fn ports() -> Vec<PortInfo> {
    imp::ports()
}

// ----- Windows -----------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::io;

    use windows_sys::Win32::Devices::Communication::{
        COMMTIMEOUTS, DCB, EVENPARITY, EscapeCommFunction, GetCommState, NOPARITY, ODDPARITY, ONESTOPBIT, SetCommState,
        SetCommTimeouts, TWOSTOPBITS,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_TYPE_PIPE, GetFileType, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    use super::{PortInfo, Settings, WAIT};

    pub struct Port {
        handle: HANDLE,
        pub serial: bool,
        pipe: bool,
    }

    // SAFETY: a file handle may be used from any thread; reads and writes on it are
    // synchronous, and the handle lives until Drop.
    unsafe impl Send for Port {}
    unsafe impl Sync for Port {}

    // The EscapeCommFunction codes.
    const SETRTS: u32 = 3;
    const CLRRTS: u32 = 4;
    const SETDTR: u32 = 5;
    const CLRDTR: u32 = 6;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl Port {
        pub fn open(path: &str, settings: &Settings) -> Result<Port, String> {
            // COM10 and above need the device namespace; it works for COM1 too.
            let full = if path.starts_with(r"\\") { path.to_owned() } else { format!(r"\\.\{path}") };
            let name = wide(&full);
            // SAFETY: a valid, NUL-terminated name; no security attributes or template.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                let e = io::Error::last_os_error();
                return Err(match e.raw_os_error() {
                    Some(2) => format!("{path} isn't there (unplugged?)"),
                    Some(5) => format!("{path} is in use by another program"),
                    _ => format!("{path}: {e}"),
                });
            }
            let mut port = Port { handle, serial: false, pipe: false };
            // SAFETY: the handle is open; DCB is plain data.
            let mut dcb: DCB = unsafe { std::mem::zeroed() };
            dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
            if unsafe { GetCommState(handle, &mut dcb) } != 0 {
                port.serial = true;
                dcb.BaudRate = settings.baud;
                dcb.ByteSize = settings.data_bits;
                dcb.Parity = match settings.parity {
                    super::Parity::None => NOPARITY,
                    super::Parity::Odd => ODDPARITY,
                    super::Parity::Even => EVENPARITY,
                };
                dcb.StopBits = if settings.two_stop_bits { TWOSTOPBITS } else { ONESTOPBIT };
                // fBinary (bit 0), DTR and RTS on or off (bits 4–5, 12–13), no flow control.
                let dtr = u32::from(settings.dtr);
                let rts = u32::from(settings.rts);
                dcb._bitfield = 1 | (dtr << 4) | (rts << 12);
                if unsafe { SetCommState(handle, &dcb) } == 0 {
                    return Err(format!(
                        "{path} won't take {} {}: {}",
                        settings.baud,
                        settings.format(),
                        io::Error::last_os_error()
                    ));
                }
                // A read comes back as soon as anything came, or after WAIT.
                let timeouts = COMMTIMEOUTS {
                    ReadIntervalTimeout: u32::MAX,
                    ReadTotalTimeoutMultiplier: u32::MAX,
                    ReadTotalTimeoutConstant: WAIT.as_millis() as u32,
                    WriteTotalTimeoutMultiplier: 0,
                    WriteTotalTimeoutConstant: 2000,
                };
                unsafe { SetCommTimeouts(handle, &timeouts) };
            } else {
                // SAFETY: the handle is open.
                port.pipe = unsafe { GetFileType(handle) } == FILE_TYPE_PIPE;
            }
            Ok(port)
        }

        pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
            let mut want = buf.len() as u32;
            if self.pipe {
                // A pipe's read would wait for data and hold up writing: look first.
                let mut avail = 0u32;
                // SAFETY: the handle is open; only the count is asked for.
                let ok = unsafe {
                    PeekNamedPipe(
                        self.handle,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut avail,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if avail == 0 {
                    std::thread::sleep(WAIT);
                    return Ok(0);
                }
                want = want.min(avail);
            }
            let mut got = 0u32;
            // SAFETY: buf is valid for `want` bytes.
            let ok = unsafe { ReadFile(self.handle, buf.as_mut_ptr(), want, &mut got, std::ptr::null_mut()) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(got as usize)
        }

        pub fn write(&self, data: &[u8]) -> io::Result<usize> {
            let mut put = 0u32;
            // SAFETY: data is valid for its length.
            let ok =
                unsafe { WriteFile(self.handle, data.as_ptr(), data.len() as u32, &mut put, std::ptr::null_mut()) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(put as usize)
        }

        pub fn set_lines(&self, dtr: bool, rts: bool) -> io::Result<()> {
            if !self.serial {
                return Ok(());
            }
            for code in [if dtr { SETDTR } else { CLRDTR }, if rts { SETRTS } else { CLRRTS }] {
                // SAFETY: the handle is an open serial port.
                if unsafe { EscapeCommFunction(self.handle, code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }

    impl Drop for Port {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed once.
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// The ports in the registry's SERIALCOMM list (what the drivers registered).
    pub fn ports() -> Vec<PortInfo> {
        let out = crate::language::lsp::discovery::quiet_command(std::path::Path::new("reg"))
            .args(["query", r"HKLM\HARDWARE\DEVICEMAP\SERIALCOMM"])
            .stdin(std::process::Stdio::null())
            .output();
        let Ok(out) = out else { return Vec::new() };
        let mut ports: Vec<PortInfo> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                let device = parts.next()?;
                if parts.next()? != "REG_SZ" {
                    return None;
                }
                let path = parts.next()?.to_owned();
                Some(PortInfo { path, description: super::describe(device) })
            })
            .collect();
        ports.sort_by_key(|p| p.path.trim_start_matches("COM").parse::<u32>().unwrap_or(u32::MAX));
        ports
    }
}

// ----- Unix --------------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::io;

    use super::{PortInfo, Settings, WAIT};

    pub struct Port {
        fd: libc::c_int,
        pub serial: bool,
    }

    fn speed(baud: u32) -> Option<libc::speed_t> {
        Some(match baud {
            300 => libc::B300,
            1200 => libc::B1200,
            2400 => libc::B2400,
            4800 => libc::B4800,
            9600 => libc::B9600,
            19_200 => libc::B19200,
            38_400 => libc::B38400,
            57_600 => libc::B57600,
            115_200 => libc::B115200,
            230_400 => libc::B230400,
            #[cfg(any(target_os = "linux", target_os = "android"))]
            460_800 => libc::B460800,
            #[cfg(any(target_os = "linux", target_os = "android"))]
            921_600 => libc::B921600,
            _ => return None,
        })
    }

    impl Port {
        pub fn open(path: &str, settings: &Settings) -> Result<Port, String> {
            let name = CString::new(path).map_err(|_| format!("{path}: not a path"))?;
            // SAFETY: a valid C string.
            let fd = unsafe {
                libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
            };
            if fd < 0 {
                let e = io::Error::last_os_error();
                return Err(match e.raw_os_error() {
                    Some(libc::ENOENT) => format!("{path} isn't there (unplugged?)"),
                    Some(libc::EACCES) => format!("{path}: no permission (on Linux, join the dialout group)"),
                    Some(libc::EBUSY) => format!("{path} is in use by another program"),
                    _ => format!("{path}: {e}"),
                });
            }
            let mut port = Port { fd, serial: false };
            // SAFETY: fd is open.
            if unsafe { libc::isatty(fd) } == 1 {
                port.serial = true;
                let Some(rate) = speed(settings.baud) else {
                    return Err(format!("{} baud isn't one this system sets", settings.baud));
                };
                // SAFETY: termios is plain data, filled by tcgetattr before use.
                let mut t: libc::termios = unsafe { std::mem::zeroed() };
                if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
                    return Err(format!("{path}: {}", io::Error::last_os_error()));
                }
                unsafe {
                    libc::cfmakeraw(&mut t);
                    libc::cfsetispeed(&mut t, rate);
                    libc::cfsetospeed(&mut t, rate);
                }
                t.c_cflag &= !(libc::CSIZE | libc::PARENB | libc::PARODD | libc::CSTOPB);
                t.c_cflag |= libc::CLOCAL
                    | libc::CREAD
                    | match settings.data_bits {
                        5 => libc::CS5,
                        6 => libc::CS6,
                        7 => libc::CS7,
                        _ => libc::CS8,
                    };
                match settings.parity {
                    super::Parity::None => {}
                    super::Parity::Even => t.c_cflag |= libc::PARENB,
                    super::Parity::Odd => t.c_cflag |= libc::PARENB | libc::PARODD,
                }
                if settings.two_stop_bits {
                    t.c_cflag |= libc::CSTOPB;
                }
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) } != 0 {
                    return Err(format!(
                        "{path} won't take {} {}: {}",
                        settings.baud,
                        settings.format(),
                        io::Error::last_os_error()
                    ));
                }
                let _ = port.set_lines(settings.dtr, settings.rts);
            }
            Ok(port)
        }

        pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
            let mut poll = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd.
            let n = unsafe { libc::poll(&mut poll, 1, WAIT.as_millis() as libc::c_int) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n == 0 {
                return Ok(0);
            }
            if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the port went away"));
            }
            // SAFETY: buf is valid for its length.
            let got = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if got < 0 {
                let e = io::Error::last_os_error();
                return if e.kind() == io::ErrorKind::WouldBlock { Ok(0) } else { Err(e) };
            }
            if got == 0 && poll.revents & libc::POLLHUP != 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the other end closed"));
            }
            Ok(got as usize)
        }

        pub fn write(&self, data: &[u8]) -> io::Result<usize> {
            loop {
                // SAFETY: data is valid for its length.
                let n = unsafe { libc::write(self.fd, data.as_ptr().cast(), data.len()) };
                if n >= 0 {
                    return Ok(n as usize);
                }
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::WouldBlock {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        pub fn set_lines(&self, dtr: bool, rts: bool) -> io::Result<()> {
            if !self.serial {
                return Ok(());
            }
            for (on, bit) in [(dtr, libc::TIOCM_DTR), (rts, libc::TIOCM_RTS)] {
                let request = if on { libc::TIOCMBIS } else { libc::TIOCMBIC };
                // SAFETY: fd is an open terminal; the bit is an int.
                if unsafe { libc::ioctl(self.fd, request as _, &bit) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }

    impl Drop for Port {
        fn drop(&mut self) {
            // SAFETY: fd is open and closed once.
            unsafe { libc::close(self.fd) };
        }
    }

    /// USB serial devices, and the first few built-in ports that exist.
    pub fn ports() -> Vec<PortInfo> {
        let mut out = Vec::new();
        let Ok(dev) = std::fs::read_dir("/dev") else { return out };
        let mut names: Vec<String> = dev.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let usb = name.starts_with("ttyUSB") || name.starts_with("ttyACM") || name.starts_with("cu.");
            // ttyS* exist for ports that may not be there; /sys says which have a device.
            let builtin =
                name.starts_with("ttyS") && std::path::Path::new(&format!("/sys/class/tty/{name}/device")).exists();
            if usb || builtin {
                let description = if name.starts_with("ttyACM") || name.starts_with("cu.usbmodem") {
                    "USB serial (CDC: Arduino, Pico…)"
                } else if usb {
                    "USB serial adapter"
                } else {
                    "Built-in serial port"
                };
                out.push(PortInfo { path: format!("/dev/{name}"), description: description.to_owned() });
            }
        }
        out
    }
}

/// What kind of device a Windows SERIALCOMM entry is, from its driver's name.
#[cfg_attr(not(windows), allow(dead_code))]
fn describe(device: &str) -> String {
    let d = device.to_ascii_lowercase();
    if d.contains("usbser") {
        "USB serial (CDC: Arduino, Pico…)"
    } else if d.contains("silabser") || d.contains("cp210") {
        "USB serial (Silicon Labs CP210x)"
    } else if d.contains("ch341") || d.contains("wch") {
        "USB serial (CH340)"
    } else if d.contains("vcp") || d.contains("ftdi") {
        "USB serial (FTDI)"
    } else if d.contains("bthmodem") || d.contains("bluetooth") {
        "Bluetooth serial"
    } else if d.contains("serial") {
        "Built-in serial port"
    } else {
        device.rsplit('\\').next().unwrap_or(device)
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_kinds() {
        assert_eq!(describe(r"\Device\USBSER000"), "USB serial (CDC: Arduino, Pico…)");
        assert_eq!(describe(r"\Device\Serial0"), "Built-in serial port");
    }

    /// A program on the other end of a named pipe, standing in for a board.
    #[cfg(windows)]
    #[test]
    fn a_pipe_stands_in_for_a_board() {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
        use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};

        let name = format!(r"\\.\pipe\coiman-serial-test-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a valid name; default security.
        let server = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(server, INVALID_HANDLE_VALUE);
        let server = server as usize;
        let board = std::thread::spawn(move || {
            let h = server as windows_sys::Win32::Foundation::HANDLE;
            unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            let mut put = 0u32;
            let hello = b"boot\r\ntemp:21.5\r\n";
            unsafe { WriteFile(h, hello.as_ptr(), hello.len() as u32, &mut put, std::ptr::null_mut()) };
            let mut buf = [0u8; 64];
            let mut got = 0u32;
            unsafe { ReadFile(h, buf.as_mut_ptr(), 64, &mut got, std::ptr::null_mut()) };
            unsafe { CloseHandle(h) };
            buf[..got as usize].to_vec()
        });
        std::thread::sleep(Duration::from_millis(100));
        let port = Port::open(&name, &Settings::default()).unwrap();
        assert!(!port.is_serial());
        port.set_lines(false, false).unwrap();
        let mut received = Vec::new();
        let mut buf = [0u8; 256];
        for _ in 0..40 {
            let n = port.read(&mut buf).unwrap();
            received.extend_from_slice(&buf[..n]);
            if received.ends_with(b"temp:21.5\r\n") {
                break;
            }
        }
        assert_eq!(received, b"boot\r\ntemp:21.5\r\n");
        port.write_all(b"ping\n").unwrap();
        assert_eq!(board.join().unwrap(), b"ping\n");
        // The board hung up: reading says so.
        let mut gone = false;
        for _ in 0..20 {
            if port.read(&mut buf).is_err() {
                gone = true;
                break;
            }
        }
        assert!(gone);
        let missing = Port::open(r"\\.\pipe\coiman-no-such-pipe", &Settings::default()).err().unwrap_or_default();
        assert!(missing.contains("isn't there"), "{missing}");
        let _ = ports();
    }
}
