use crate::config::Config;
use crate::guard::{Action, Fault, Guard, Phase};
use crate::log;
use crate::power::PowerCap;
use crate::sensor::{Reading, Sensor};
use std::time::Instant;

const IDLE_PIN_AMPS: f32 = 1.0;
const LOADED_BOARD_WATTS: f32 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

pub trait Host {
    fn notify(&mut self, severity: Severity, title: &str, body: &str);
    fn shutdown(&mut self, reason: &str) -> Result<(), String>;
}

pub struct Warden {
    cfg: Config,
    guard: Guard,
    sensor: Box<dyn Sensor>,
    cap: PowerCap,
    hold_error: bool,
}

impl Warden {
    pub fn new(cfg: Config, sensor: Box<dyn Sensor>, cap: PowerCap) -> Self {
        if cfg.power_cap.enabled
            && let Err(e) = cap.available()
        {
            log!("power cap unavailable: {e}");
        }
        Self {
            guard: Guard::new(cfg.clone()),
            cfg,
            sensor,
            cap,
            hold_error: false,
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn phase(&self) -> Phase {
        self.guard.phase()
    }

    pub fn summary(&self) -> String {
        let state = match self.guard.phase() {
            Phase::Normal => "OK",
            Phase::Warning { .. } => "WARNING",
            Phase::Recovering { .. } => "Recovering",
            Phase::Blind => "Sensor unreadable",
            Phase::Halted => "SHUTDOWN",
        };
        let capped = if self.cap.engaged() { " · capped" } else { "" };
        match self.guard.last() {
            Some(r) => {
                let (pin, amps) = r.hottest();
                format!("{state}{capped} · pin {} {amps:.1} A · {:.0} W", pin + 1, r.watts())
            }
            None => format!("{state}{capped}"),
        }
    }

    fn sample(&mut self) -> Result<Reading, String> {
        let reading = self.sensor.read()?;
        if reading.total() < IDLE_PIN_AMPS
            && let Some(watts) = self.cap.watts().filter(|w| *w > LOADED_BOARD_WATTS)
        {
            return Err(format!(
                "pins read {:.1} A while the board draws {watts:.0} W",
                reading.total()
            ));
        }
        Ok(reading)
    }

    pub fn tick(&mut self, host: &mut impl Host) -> Option<Fault> {
        let sample = self.sample();
        let action = self.guard.step(Instant::now(), sample);
        self.hold(host);
        match action? {
            Action::Warn(fault) => {
                let cap = self.engage();
                let deadline = if self.cfg.shutdown {
                    format!(" Shutdown in {} s unless it recovers.", self.cfg.recover_secs)
                } else {
                    String::new()
                };
                self.emit(
                    host,
                    Severity::Warn,
                    "12V-2x6 warning",
                    &format!("{fault}.{cap}{deadline}"),
                );
            }
            Action::Recovered => {
                let body = match self.cap.engaged() {
                    true => format!(
                        "Back within limits. Power cap held for {} s.",
                        self.cfg.power_cap.restore_after_secs
                    ),
                    false => "Back within limits.".to_owned(),
                };
                self.emit(host, Severity::Info, "12V-2x6 recovered", &body);
            }
            Action::Restore => match self.cap.release() {
                Ok(Some(mw)) => self.emit(
                    host,
                    Severity::Info,
                    "Power limit restored",
                    &format!("{} W", mw / 1000),
                ),
                Ok(None) => {}
                Err(e) => self.emit(host, Severity::Warn, "Power limit restore failed", &e),
            },
            Action::SensorLost => {
                let cap = self.engage();
                let body = format!("Pin currents cannot be read.{cap}");
                self.emit(host, Severity::Warn, "12V-2x6 sensor lost", &body);
            }
            Action::SensorBack => self.emit(host, Severity::Info, "12V-2x6 sensor back", "Readings resumed."),
            Action::Shutdown(fault) => {
                let reason = format!("{fault}. Shutting down to protect the connector.");
                self.emit(host, Severity::Crit, "12V-2x6 CRITICAL", &reason);
                if let Err(e) = host.shutdown(&reason) {
                    log!("shutdown failed: {e}");
                }
                return Some(fault);
            }
        }
        None
    }

    fn engage(&mut self) -> String {
        match self.cfg.power_cap.enabled.then(|| self.cap.engage()) {
            Some(Ok(mw)) => format!(" Power limited to {} W.", mw / 1000),
            Some(Err(e)) => format!(" Power cap failed: {e}."),
            None => String::new(),
        }
    }

    fn hold(&mut self, host: &mut impl Host) {
        match self.cap.hold() {
            Ok(()) => self.hold_error = false,
            Err(e) if !self.hold_error => {
                self.hold_error = true;
                self.emit(host, Severity::Warn, "Power cap lost", &e);
            }
            Err(_) => {}
        }
    }

    fn emit(&self, host: &mut impl Host, severity: Severity, title: &str, body: &str) {
        log!("[{severity:?}] {title}: {body} ({})", self.summary());
        host.notify(severity, title, body);
    }
}
