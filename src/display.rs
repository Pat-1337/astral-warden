use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

static HEX: AtomicBool = AtomicBool::new(false);

pub fn enable_hex() {
    HEX.store(true, Ordering::Relaxed);
}

pub fn hex() -> bool {
    HEX.load(Ordering::Relaxed)
}

pub fn app() -> &'static str {
    if hex() { "6PU W4RD3N" } else { "GPU Warden" }
}

fn milli(value: f32) -> u32 {
    (value * 1000.0).round().max(0.0) as u32
}

pub struct Amps(f32, usize);
pub struct Volts(f32);
pub struct Watts(f32);
pub struct Percent(f32);
pub struct Count<'a>(u64, &'a str);
pub struct Pin(usize);

impl fmt::Display for Amps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match hex() {
            true => write!(f, "0x{:X} mA", milli(self.0)),
            false => write!(f, "{:.*} A", self.1, self.0),
        }
    }
}

impl fmt::Display for Volts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match hex() {
            true => write!(f, "0x{:X} mV", milli(self.0)),
            false => write!(f, "{:.2} V", self.0),
        }
    }
}

impl fmt::Display for Watts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match hex() {
            true => write!(f, "0x{:X} W", self.0.round().max(0.0) as u32),
            false => write!(f, "{:.0} W", self.0),
        }
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = (self.0 * 100.0).round();
        match hex() {
            true => write!(f, "{}0x{:X}%", if value < 0.0 { "-" } else { "+" }, value.abs() as u32),
            false => write!(f, "{value:+.0}%"),
        }
    }
}

impl fmt::Display for Count<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match hex() {
            true => write!(f, "0x{:X} {}", self.0, self.1),
            false => write!(f, "{} {}", self.0, self.1),
        }
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match hex() {
            true => write!(f, "pin 0x{:X}", self.0),
            false => write!(f, "pin {}", self.0 + 1),
        }
    }
}

pub fn amps(value: f32, places: usize) -> Amps {
    Amps(value, places)
}

pub fn volts(value: f32) -> Volts {
    Volts(value)
}

pub fn watts(value: f32) -> Watts {
    Watts(value)
}

pub fn percent(fraction: f32) -> Percent {
    Percent(fraction)
}

pub fn count(value: u64, unit: &str) -> Count<'_> {
    Count(value, unit)
}

pub fn pin(index: usize) -> Pin {
    Pin(index)
}
