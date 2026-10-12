use std::sync::atomic::{AtomicBool, Ordering};

static HEX: AtomicBool = AtomicBool::new(false);

pub fn enable_hex() {
    HEX.store(true, Ordering::Relaxed);
}

pub fn hex() -> bool {
    HEX.load(Ordering::Relaxed)
}

pub fn app() -> &'static str {
    if hex() { "4STR4L W4RD3N" } else { "Astral Warden" }
}

pub fn amps(value: f32, places: usize) -> String {
    match hex() {
        true => format!("0x{:X} mA", (value * 1000.0).round().max(0.0) as u32),
        false => format!("{value:.places$} A"),
    }
}

pub fn volts(value: f32) -> String {
    match hex() {
        true => format!("0x{:X} mV", (value * 1000.0).round().max(0.0) as u32),
        false => format!("{value:.2} V"),
    }
}

pub fn watts(value: f32) -> String {
    match hex() {
        true => format!("0x{:X} W", value.round().max(0.0) as u32),
        false => format!("{value:.0} W"),
    }
}

pub fn percent(fraction: f32) -> String {
    let value = (fraction * 100.0).round();
    match hex() {
        true => format!("{}0x{:X}%", if value < 0.0 { "-" } else { "+" }, value.abs() as u32),
        false => format!("{value:+.0}%"),
    }
}

pub fn count(value: u64, unit: &str) -> String {
    match hex() {
        true => format!("0x{value:X} {unit}"),
        false => format!("{value} {unit}"),
    }
}

pub fn pin(index: usize) -> String {
    match hex() {
        true => format!("pin 0x{index:X}"),
        false => format!("pin {}", index + 1),
    }
}
