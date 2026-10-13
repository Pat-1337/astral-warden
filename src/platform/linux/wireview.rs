use super::read_hex;
use crate::sensor::{PINS, Reading, Sensor};
use crate::wireview::{self, Port, USB_PRODUCT, USB_VENDOR, WireView};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const TTYS: &str = "/sys/class/tty";
const HWMON: &str = "/sys/class/hwmon";
const HWMON_NAME: &str = "wireview";
const TIMEOUT_DECISECONDS: u8 = 10;

pub type Opened = (Box<dyn Sensor>, &'static str, String);

struct Tty(File);

impl Tty {
    fn open(path: &Path) -> Result<Self, String> {
        let fail = |e: std::io::Error| format!("{}: {e}", path.display());
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(path)
            .map_err(fail)?;

        let fd = file.as_raw_fd();
        let mut tio = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut tio) } != 0 {
            return Err(fail(std::io::Error::last_os_error()));
        }

        unsafe {
            libc::cfmakeraw(&mut tio);
            libc::cfsetspeed(&mut tio, libc::B115200);
        }
        tio.c_cflag |= libc::CLOCAL | libc::CREAD;
        tio.c_cc[libc::VMIN] = 0;
        tio.c_cc[libc::VTIME] = TIMEOUT_DECISECONDS;

        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &tio) } != 0 || unsafe { libc::ioctl(fd, libc::TIOCEXCL) } != 0 {
            return Err(fail(std::io::Error::last_os_error()));
        }

        Ok(Self(file))
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        unsafe { libc::ioctl(self.0.as_raw_fd(), libc::TIOCNXCL) };
    }
}

impl Port for Tty {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.0.write_all(data).map_err(|e| format!("WireView write: {e}"))
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        self.0.read_exact(buf).map_err(|e| match e.kind() {
            ErrorKind::UnexpectedEof => "WireView did not answer".into(),
            _ => format!("WireView read: {e}"),
        })
    }

    fn discard(&mut self) {
        unsafe { libc::tcflush(self.0.as_raw_fd(), libc::TCIFLUSH) };
    }
}

struct Hwmon {
    pins: [(PathBuf, PathBuf); PINS],
    text: String,
}

impl Hwmon {
    fn new(dir: &Path) -> Self {
        Self {
            pins: std::array::from_fn(|pin| {
                (
                    dir.join(format!("in{pin}_input")),
                    dir.join(format!("curr{}_input", pin + 1)),
                )
            }),
            text: String::with_capacity(16),
        }
    }

    fn milli(text: &mut String, path: &Path) -> Result<f32, String> {
        text.clear();
        File::open(path)
            .and_then(|mut f| f.read_to_string(text))
            .map_err(|e| match e.raw_os_error() {
                Some(libc::ENODATA) => "wireviewd is not sending data".into(),
                _ => format!("{}: {e}", path.display()),
            })?;

        let value: f32 = text.trim().parse().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(value / 1000.0)
    }
}

impl Sensor for Hwmon {
    fn read(&mut self) -> Result<Reading, String> {
        let mut reading = Reading::default();

        for (pin, (volts, amps)) in self.pins.iter().enumerate() {
            reading.volts[pin] = Self::milli(&mut self.text, volts)?;
            reading.amps[pin] = Self::milli(&mut self.text, amps)?;
        }

        if reading.plausible() {
            Ok(reading)
        } else {
            Err(format!("implausible reading {reading}"))
        }
    }
}

fn hwmon() -> Option<PathBuf> {
    fs::read_dir(HWMON)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|dir| fs::read_to_string(dir.join("name")).is_ok_and(|n| n.trim() == HWMON_NAME))
}

fn ttys() -> impl Iterator<Item = PathBuf> {
    fs::read_dir(TTYS)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("ttyACM")))
        .filter(|e| {
            let usb = e.path().join("device/..");
            read_hex(&usb, "idVendor") == Some(USB_VENDOR) && read_hex(&usb, "idProduct") == Some(USB_PRODUCT)
        })
        .map(|e| Path::new("/dev").join(e.file_name()))
}

fn probe(path: &Path) -> Result<(Tty, &'static str), String> {
    Tty::open(path).and_then(wireview::probe)
}

fn reopen() -> Result<Tty, String> {
    ttys()
        .find_map(|path| probe(&path).ok())
        .map(|(tty, _)| tty)
        .ok_or_else(|| "WireView Pro II unplugged".into())
}

fn sensor(tty: Tty, model: &'static str, path: &Path, reopen: impl FnMut() -> Result<Tty, String> + 'static) -> Opened {
    (
        Box::new(WireView::new(tty, reopen)),
        model,
        format!("via {}", path.display()),
    )
}

pub fn open(port: Option<&Path>) -> Option<Result<Opened, String>> {
    if let Some(path) = port {
        let owned = path.to_owned();
        let reopen = move || probe(&owned).map(|(tty, _)| tty);
        return Some(probe(path).map(|(tty, model)| sensor(tty, model, path, reopen)));
    }

    if let Some(dir) = hwmon() {
        let mut sensor = Hwmon::new(&dir);
        let detail = format!("via {} (wireviewd)", dir.display());
        return Some(
            sensor
                .read()
                .map(|_| (Box::new(sensor) as Box<dyn Sensor>, "WireView Pro II", detail)),
        );
    }

    let mut failure = None;
    for path in ttys() {
        match probe(&path) {
            Ok((tty, model)) => return Some(Ok(sensor(tty, model, &path, reopen))),
            Err(e) => failure = Some(Err(e)),
        }
    }

    failure
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hwmon() {
        let dir = std::env::temp_dir().join(format!("astral-warden-hwmon-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        for pin in 0..PINS {
            fs::write(dir.join(format!("in{pin}_input")), "12010\n").unwrap();
            fs::write(
                dir.join(format!("curr{}_input", pin + 1)),
                format!("{}\n", 8000 + pin * 100),
            )
            .unwrap();
        }

        let reading = Hwmon::new(&dir).read();
        let _ = fs::remove_dir_all(&dir);

        let reading = reading.unwrap();
        assert_eq!(reading.amps[5], 8.5);
        assert_eq!(reading.volts[0], 12.01);
    }
}
