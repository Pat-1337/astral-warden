use crate::sensor::{PINS, Reading, Sensor};

pub const USB_VENDOR: u16 = 0x0483;
pub const USB_PRODUCT: u16 = 0x5740;

const VENDOR: u8 = 0xEF;
const PRODUCTS: &[(u8, &str)] = &[(5, "WireView Pro II"), (6, "WireView Pro II Noctua Edition")];
const READ_VENDOR: u8 = 0x01;
const READ_SENSORS: u8 = 0x04;
const FRAME_LEN: usize = 100;
const FAN_AT: usize = 10;
const PINS_AT: usize = 12;
const PIN_LEN: usize = 12;
const PAD_AT: [usize; 2] = [11, 95];

pub trait Port: Sized {
    fn write(&mut self, data: &[u8]) -> Result<(), String>;

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String>;

    fn discard(&mut self);
}

pub fn decode(frame: &[u8; FRAME_LEN]) -> Result<Reading, String> {
    if frame[FAN_AT] > 100 || PAD_AT.iter().any(|&i| frame[i] != 0) {
        return Err("corrupt frame".into());
    }

    let mut reading = Reading::default();
    for pin in 0..PINS {
        let base = PINS_AT + pin * PIN_LEN;
        let millivolts = i16::from_le_bytes([frame[base], frame[base + 1]]);
        let milliamps = u32::from_le_bytes([frame[base + 4], frame[base + 5], frame[base + 6], frame[base + 7]]);

        reading.volts[pin] = f32::from(millivolts) / 1000.0;
        reading.amps[pin] = milliamps as f32 / 1000.0;
    }

    if reading.plausible() {
        Ok(reading)
    } else {
        Err(format!("implausible frame {reading}"))
    }
}

pub fn probe<P: Port>(mut port: P) -> Result<(P, &'static str), String> {
    let mut ids = [0; 3];
    port.discard();
    port.write(&[READ_VENDOR])?;
    port.read_exact(&mut ids)?;

    let model = PRODUCTS
        .iter()
        .find(|p| ids[0] == VENDOR && p.0 == ids[1])
        .ok_or_else(|| format!("not a WireView Pro II (vendor {:#04x}, product {})", ids[0], ids[1]))?
        .1;

    Ok((port, model))
}

pub struct WireView<P: Port, F: FnMut() -> Result<P, String>> {
    port: Option<P>,
    open: F,
}

impl<P: Port, F: FnMut() -> Result<P, String>> WireView<P, F> {
    pub fn new(port: P, open: F) -> Self {
        Self { port: Some(port), open }
    }

    fn frame(port: &mut P) -> Result<Reading, String> {
        let mut frame = [0; FRAME_LEN];
        port.write(&[READ_SENSORS])?;
        port.read_exact(&mut frame)?;
        decode(&frame)
    }
}

impl<P: Port, F: FnMut() -> Result<P, String>> Sensor for WireView<P, F> {
    fn read(&mut self) -> Result<Reading, String> {
        let port = match &mut self.port {
            Some(port) => port,
            None => self.port.insert(probe((self.open)()?)?.0),
        };

        match Self::frame(port) {
            Ok(reading) => Ok(reading),
            Err(_) => {
                port.discard();
                Self::frame(port).inspect_err(|_| self.port = None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pins: [(i16, u32); PINS]) -> [u8; FRAME_LEN] {
        let mut frame = [0; FRAME_LEN];
        frame[FAN_AT] = 42;

        for (pin, (mv, ma)) in pins.into_iter().enumerate() {
            let base = PINS_AT + pin * PIN_LEN;
            frame[base..base + 2].copy_from_slice(&mv.to_le_bytes());
            frame[base + 4..base + 8].copy_from_slice(&ma.to_le_bytes());
        }

        frame
    }

    #[test]
    fn decodes_pins() {
        let reading = decode(&frame([
            (12_050, 8_000),
            (12_000, 8_100),
            (11_990, 7_900),
            (12_010, 8_000),
            (12_000, 9_600),
            (12_020, 7_500),
        ]))
        .unwrap();

        assert_eq!(reading.amps[4], 9.6);
        assert_eq!(reading.volts[0], 12.05);
    }

    #[test]
    fn rejects_corrupt_frames() {
        let mut bad = frame([(12_000, 8_000); PINS]);
        bad[FAN_AT] = 105;
        assert!(decode(&bad).is_err());

        let mut bad = frame([(12_000, 8_000); PINS]);
        bad[PAD_AT[0]] = 122;
        assert!(decode(&bad).is_err());

        assert!(decode(&frame([(12_000, 61_000); PINS])).is_err());
    }
}
