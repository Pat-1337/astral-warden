use crate::config::{self, Config};
use crate::display;
use crate::guard::{Action, Guard, Phase, imbalance};
use crate::log;
use crate::power::PowerCap;
use crate::sensor::{Reading, Sensor};
use std::time::{Duration, Instant};

const IDLE_PIN_AMPS: f32 = 1.0;
const LOADED_BOARD_WATTS: f32 = 150.0;
const SHUTDOWN_RETRY: Duration = Duration::from_secs(15);
const BOARD_INTERVAL: Duration = Duration::from_secs(5);
const ALARM_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

pub trait Host {
    fn notify(&mut self, severity: Severity, title: &str, body: &str);

    fn alert(&mut self, title: &str, body: &str) {
        self.notify(Severity::Warn, title, body);
    }

    fn alarm(&mut self, severity: Severity);

    fn shutdown(&mut self, reason: &str) -> Result<(), String>;
}

pub struct Warden {
    name: String,
    cfg: Config,
    guard: Guard,
    sensor: Box<dyn Sensor>,
    cap: PowerCap,
    hold_error: bool,
    release_error: bool,
    shutdown: Option<(String, Instant)>,
    board: Option<(Instant, Option<f32>)>,
    last_error: Option<String>,
    alarm_at: Option<Instant>,
    note: Option<String>,
}

fn take_note() -> Option<String> {
    let path = config::note_path();
    let note = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);

    Some(note.trim().to_owned()).filter(|n| !n.is_empty())
}

fn save_note(reason: &str) {
    let path = config::note_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let when = log::timestamp().replace('T', " ").replace('Z', " UTC");
    let _ = std::fs::write(path, format!("{when}\n{reason}"));
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
}

impl Warden {
    pub fn new(name: String, cfg: Config, sensor: Box<dyn Sensor>, cap: PowerCap) -> Self {
        if cfg.power_cap.enabled
            && let Err(e) = cap.available()
        {
            log!("power cap unavailable: {e}");
        }

        Self {
            name,
            guard: Guard::new(cfg.clone()),
            cfg,
            sensor,
            cap,
            hold_error: false,
            release_error: false,
            shutdown: None,
            board: None,
            last_error: None,
            alarm_at: None,
            note: take_note(),
        }
    }

    #[cfg_attr(not(any(windows, feature = "omarchy")), allow(dead_code))]
    pub fn phase(&self) -> Phase {
        self.guard.phase()
    }

    #[cfg_attr(not(any(windows, feature = "omarchy")), allow(dead_code))]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn greeting(&self) -> (Severity, String, String) {
        if self.guard.last().is_some() {
            return (
                Severity::Info,
                format!("{} is running", display::app()),
                format!("{} · pins OK", self.name),
            );
        }

        if let Some(e) = &self.last_error {
            log!("first sensor read failed: {e}");
        }

        (
            Severity::Warn,
            display::app().into(),
            format!("{} found, but its pins can't be read.", self.name),
        )
    }

    #[cfg(feature = "omarchy")]
    pub fn hottest(&self) -> Option<(usize, f32)> {
        self.guard.last().map(|r| r.hottest())
    }

    fn state(&self) -> &'static str {
        match self.guard.phase() {
            Phase::Normal => "OK",
            Phase::Warning { .. } => "WARNING",
            Phase::Recovering { .. } => "Recovering",
            Phase::Blind => "Sensor unreadable",
            Phase::Halted => "SHUTDOWN",
        }
    }

    #[cfg_attr(not(any(windows, feature = "omarchy")), allow(dead_code))]
    pub fn details(&self) -> Vec<(String, String)> {
        let mut rows = vec![("State".to_owned(), self.state().to_owned())];

        match self.guard.last() {
            Some(r) => {
                rows.extend(r.amps.iter().zip(r.volts).enumerate().map(|(pin, (amps, volts))| {
                    let value = format!("{}   {}", display::amps(*amps, 2), display::volts(volts));
                    (capitalized(&display::pin(pin)), value)
                }));

                rows.push((
                    "Total".into(),
                    format!("{}   {}", display::amps(r.total(), 1), display::watts(r.watts())),
                ));

                if r.total() >= self.cfg.imbalance_min_total_amps {
                    let (pin, deviation) = imbalance(&r);
                    rows.push((
                        "Balance".into(),
                        format!("{} {}", display::pin(pin), display::percent(deviation)),
                    ));
                }
            }
            None => rows.push(("Pins".into(), "no reading".into())),
        }

        if let (Some(watts), Some(limit)) = (self.cap.watts(), self.cap.limit_watts()) {
            rows.push((
                "Board".into(),
                format!("{} / {}", display::watts(watts), display::watts(limit)),
            ));
        }

        let percent = display::percent(self.cfg.power_cap.percent as f32 / 100.0);
        let percent = percent.trim_start_matches('+');
        let cap = match (self.cfg.power_cap.enabled, self.cap.available(), self.cap.engaged()) {
            (false, _, _) => "disabled".to_owned(),
            (true, Err(_), _) => "unavailable".to_owned(),
            (true, Ok(()), true) => format!("engaged at {percent}"),
            (true, Ok(()), false) => format!("armed ({percent})"),
        };
        rows.push(("Power cap".into(), cap));

        let shutdown = if self.cfg.shutdown { "armed" } else { "disabled" };
        rows.push(("Shutdown".into(), shutdown.into()));

        rows
    }

    pub fn summary(&self) -> String {
        let state = self.state();
        let capped = if self.cap.engaged() { " · capped" } else { "" };

        match self.guard.last() {
            Some(r) => {
                let (pin, amps) = r.hottest();
                format!(
                    "{state}{capped} · {} {} · {}",
                    display::pin(pin),
                    display::amps(amps, 1),
                    display::watts(r.watts())
                )
            }
            None => format!("{state}{capped}"),
        }
    }

    fn board_watts(&mut self) -> Option<f32> {
        match self.board {
            Some((at, watts)) if at.elapsed() < BOARD_INTERVAL => watts,
            _ => {
                let watts = self.cap.watts();
                self.board = Some((Instant::now(), watts));
                watts
            }
        }
    }

    fn sample(&mut self) -> Result<Reading, String> {
        let reading = self.sensor.read()?;

        if reading.total() < IDLE_PIN_AMPS
            && let Some(watts) = self.board_watts().filter(|w| *w > LOADED_BOARD_WATTS)
        {
            return Err(format!(
                "pins read {:.1} A while the board draws {watts:.0} W",
                reading.total()
            ));
        }

        Ok(reading)
    }

    pub fn tick(&mut self, host: &mut impl Host) {
        if let Some(note) = self.note.take() {
            let body = match note.split_once('\n') {
                Some((when, reason)) => format!("Last session was shut down at {when}: {reason}."),
                None => format!("Last session was shut down: {note}."),
            };
            self.emit(host, Severity::Warn, "Previous shutdown", &body);
            host.alert(display::app(), &body);
        }

        if self.guard.phase() == Phase::Halted {
            self.hold(host);
            self.sound(host);
            self.retry_shutdown(host);
            return;
        }

        let sample = self.sample();
        self.last_error = sample.as_ref().err().cloned();
        let action = self.guard.step(Instant::now(), sample);
        self.hold(host);

        match action {
            None => {}

            Some(Action::Warn(fault)) => {
                let cap = self.engage();
                let deadline = match self.cfg.shutdown {
                    true => format!(
                        " Shutdown in {} unless it recovers.",
                        display::count(self.cfg.recover_secs, "s")
                    ),
                    false => String::new(),
                };

                let body = format!("{}.{cap}{deadline}", capitalized(&fault.to_string()));
                self.emit(host, Severity::Warn, "12V-2x6 warning", &body);
            }

            Some(Action::Critical(fault)) => {
                let cap = self.engage();
                let body = format!(
                    "{}.{cap} Shutdown is disabled in the config.",
                    capitalized(&fault.to_string())
                );
                self.emit(host, Severity::Crit, "12V-2x6 CRITICAL", &body);
            }

            Some(Action::Recovered) => {
                let window = display::count(self.cfg.power_cap.restore_after_secs, "s");
                let cap = match self.cap.engaged() {
                    true => format!(" Power cap held for {window}."),
                    false => String::new(),
                };
                let rule = match self.cfg.shutdown {
                    true => format!(" A new warning within {window} shuts down."),
                    false => String::new(),
                };

                let body = format!("Back within limits.{cap}{rule}");
                self.emit(host, Severity::Info, "12V-2x6 recovered", &body);
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

                let fault = capitalized(&fault.to_string());
                save_note(&fault);

                let reason = format!("{fault}. Shutting down to protect the connector.");
                self.emit(host, Severity::Crit, "12V-2x6 CRITICAL", &reason);

                self.shutdown = Some((reason, Instant::now()));
                self.attempt_shutdown(host);
            }
        }

        self.sound(host);

        if self.guard.phase() == Phase::Normal && self.cap.engaged() {
            self.restore(host);
        }
    }

    fn sound(&mut self, host: &mut impl Host) {
        let severity = match self.guard.phase() {
            Phase::Warning { escalated: true, .. } | Phase::Halted => Severity::Crit,
            Phase::Warning { .. } => Severity::Warn,
            _ => {
                self.alarm_at = None;
                return;
            }
        };

        if self.cfg.alarm && self.alarm_at.is_none_or(|at| at.elapsed() >= ALARM_INTERVAL) {
            self.alarm_at = Some(Instant::now());
            host.alarm(severity);
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
                let body = display::watts(mw as f32 / 1000.0);
                self.emit(host, Severity::Info, "Power limit restored", &body);
            }

            Ok(None) => {}
            Err(e) if !self.release_error => {
                self.release_error = true;
                let body = format!("{e}. Retrying.");
                self.emit(host, Severity::Warn, "Power limit restore failed", &body);
            }

            Err(_) => {}
        }
    }

    fn engage(&mut self) -> String {
        match self.cfg.power_cap.enabled.then(|| self.cap.engage()) {
            Some(Ok(mw)) => format!(" Power limited to {}.", display::watts(mw as f32 / 1000.0)),
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
        if severity != Severity::Info && log::arm() {
            log!("watching {}", self.name);
        }

        log!("[{severity:?}] {title}: {body} ({})", self.summary());
        host.notify(severity, title, body);
    }
}
