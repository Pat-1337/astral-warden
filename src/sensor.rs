use std::path::PathBuf;

pub const PINS: usize = 6;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Reading {
    pub volts: [f32; PINS],
    pub amps: [f32; PINS],
}

impl Reading {
    pub fn total(&self) -> f32 {
        self.amps.iter().sum()
    }

    pub fn hottest(&self) -> (usize, f32) {
        self.amps
            .iter()
            .copied()
            .enumerate()
            .fold((0, f32::MIN), |best, cur| if cur.1 > best.1 { cur } else { best })
    }

    pub fn watts(&self) -> f32 {
        self.volts.iter().zip(&self.amps).map(|(v, a)| v * a).sum()
    }

    pub fn plausible(&self) -> bool {
        self.volts.iter().all(|v| (6.0..=16.0).contains(v)) && self.amps.iter().all(|a| (0.0..=30.0).contains(a))
    }
}

impl std::fmt::Display for Reading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, a) in self.amps.iter().enumerate() {
            write!(f, "{}{a:.2}", if i == 0 { "" } else { " " })?;
        }
        write!(f, " | {:.1} A {:.0} W", self.total(), self.watts())
    }
}

pub trait Sensor {
    fn read(&mut self) -> Result<Reading, String>;
}

pub struct Simulated(pub PathBuf);

impl Sensor for Simulated {
    fn read(&mut self) -> Result<Reading, String> {
        let text = std::fs::read_to_string(&self.0).map_err(|e| format!("{}: {e}", self.0.display()))?;
        let mut amps = [0.0; PINS];
        let mut values = text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty());
        for slot in &mut amps {
            *slot = values
                .next()
                .ok_or("expected 6 values")?
                .parse()
                .map_err(|e| format!("bad value: {e}"))?;
        }
        Ok(Reading {
            volts: [12.0; PINS],
            amps,
        })
    }
}
