use crate::config::Config;
use crate::guard::{Action, Guard, Phase};
use crate::log;
use crate::power::PowerCap;
use crate::sensor::{Reading, Sensor};
use std::time::{Duration, Instant};

const IDLE_PIN_AMPS: f32 = 1.0;
const LOADED_BOARD_WATTS: f32 = 150.0;
const SHUTDOWN_RETRY: Duration = Duration::from_secs(15);

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
    release_error: bool,
    shutdown: Option<(String, Instant)>,
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
            release_error: false,
            shutdown: None,
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

    pub fn tick(&mut self, host: &mut impl Host) {
        if self.guard.phase() == Phase::Halted {
            self.hold(host);
            self.retry_shutdown(host);
            return;
        }
        let sample = self.sample();
        let action = self.guard.step(Instant::now(), sample);
        self.hold(host);
        match action {
            None => {}
            Some(Action::Warn(fault)) => {
                let cap = self.engage();
                let deadline = match self.cfg.shutdown {
                    true => format!(" Shutdown in {} s unless it recovers.", self.cfg.recover_secs),
                    false => String::new(),
                };
                self.emit(
                    host,
                    Severity::Warn,
                    "12V-2x6 warning",
                    &format!("{fault}.{cap}{deadline}"),
                );
            }
            Some(Action::Critical(fault)) => {
                let cap = self.engage();
                let body = format!("{fault}.{cap} Shutdown is disabled in the config.");
                self.emit(host, Severity::Crit, "12V-2x6 CRITICAL", &body);
            }
            Some(Action::Recovered) => {
                let window = self.cfg.power_cap.restore_after_secs;
                let cap = match self.cap.engaged() {
                    true => format!(" Power cap held for {window} s."),
                    false => String::new(),
                };
                let rule = match self.cfg.shutdown {
                    true => format!(" A new warning within {window} s shuts down."),
                    false => String::new(),
                };
                self.emit(
                    host,
                    Severity::Info,
                    "12V-2x6 recovered",
                    &format!("Back within limits.{cap}{rule}"),
                );
            }
            Some(Action::SensorLost) => {
                let cap = self.engage();
                let body = format!("Pin currents cannot be read.{cap}");
                self.emit(host, Severity::Warn, "12V-2x6 sensor lost", &body);
            }
            Some(Action::SensorBack) => self.emit(host, Severity::Info, "12V-2x6 sensor back", "Readings resumed."),
            Some(Action::Shutdown(fault)) => {
                let _ = self.cap.engage();
                self.cap.keep();
                let reason = format!("{fault}. Shutting down to protect the connector.");
                self.emit(host, Severity::Crit, "12V-2x6 CRITICAL", &reason);
                self.shutdown = Some((reason, Instant::now()));
                self.attempt_shutdown(host);
            }
        }
        if self.guard.phase() == Phase::Normal && self.cap.engaged() {
            self.restore(host);
        }
    }

    fn attempt_shutdown(&mut self, host: &mut impl Host) {
        let Some((reason, at)) = self.shutdown.as_mut() else {
            return;
        };
        *at = Instant::now();
        match host.shutdown(reason) {
            Ok(()) => self.shutdown = None,
            Err(e) => log!("shutdown failed, retrying in {} s: {e}", SHUTDOWN_RETRY.as_secs()),
        }
    }

    fn retry_shutdown(&mut self, host: &mut impl Host) {
        if self
            .shutdown
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= SHUTDOWN_RETRY)
        {
            self.attempt_shutdown(host);
        }
    }

    fn restore(&mut self, host: &mut impl Host) {
        match self.cap.release() {
            Ok(Some(mw)) => {
                self.release_error = false;
                self.emit(
                    host,
                    Severity::Info,
                    "Power limit restored",
                    &format!("{} W", mw / 1000),
                );
            }
            Ok(None) => {}
            Err(e) if !self.release_error => {
                self.release_error = true;
                self.emit(
                    host,
                    Severity::Warn,
                    "Power limit restore failed",
                    &format!("{e}. Retrying."),
                );
            }
            Err(_) => {}
        }
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
