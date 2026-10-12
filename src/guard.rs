use crate::config::Config;
use crate::sensor::{PINS, Reading};
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Warn,
    Crit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    Pin { pin: usize, amps: f32 },
    Imbalance { pin: usize, deviation: f32 },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin { pin, amps } => write!(f, "pin {} at {amps:.1} A", pin + 1),
            Self::Imbalance { pin, deviation } => {
                write!(f, "pin {} is {:+.0}% off the average", pin + 1, deviation * 100.0)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Warn(Fault),
    Critical(Fault),
    Recovered,
    Shutdown(Fault),
    SensorLost,
    SensorBack,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Normal,
    Warning {
        since: Instant,
        fault: Fault,
        escalated: bool,
    },
    Recovering {
        since: Instant,
        strict: bool,
    },
    Blind,
    Halted,
}

const CALM: f32 = 0.95;

pub fn imbalance(r: &Reading) -> (usize, f32) {
    let mean = r.total() / PINS as f32;
    r.amps
        .iter()
        .map(|a| (a - mean) / mean)
        .enumerate()
        .fold(
            (0, 0.0),
            |best, cur| if cur.1.abs() > best.1.abs() { cur } else { best },
        )
}

pub fn assess(r: &Reading, cfg: &Config) -> Option<(Level, Fault)> {
    let grade = |value: f32, limits: crate::config::Limits| match value {
        v if v >= limits.crit => Some(Level::Crit),
        v if v >= limits.warn => Some(Level::Warn),
        _ => None,
    };
    let (pin, amps) = r.hottest();
    let by_pin = grade(amps, cfg.pin_amps).map(|l| (l, Fault::Pin { pin, amps }));
    let by_balance = (r.total() >= cfg.imbalance_min_total_amps)
        .then(|| imbalance(r))
        .and_then(|(pin, deviation)| {
            grade(deviation.abs(), cfg.imbalance).map(|l| (l, Fault::Imbalance { pin, deviation }))
        });
    match (by_pin, by_balance) {
        (Some(p), Some(b)) if b.0 > p.0 => Some(b),
        (p, b) => p.or(b),
    }
}

pub fn calm(r: &Reading, cfg: &Config) -> bool {
    let balanced = r.total() < cfg.imbalance_min_total_amps || imbalance(r).1.abs() < cfg.imbalance.warn * CALM;
    r.hottest().1 < cfg.pin_amps.warn * CALM && balanced
}

pub struct Guard {
    cfg: Config,
    phase: Phase,
    warn_streak: u32,
    crit_streak: u32,
    ok_streak: u32,
    fail_streak: u32,
    last: Option<Reading>,
}

impl Guard {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            phase: Phase::Normal,
            warn_streak: 0,
            crit_streak: 0,
            ok_streak: 0,
            fail_streak: 0,
            last: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn last(&self) -> Option<Reading> {
        self.last
    }

    pub fn step(&mut self, now: Instant, sample: Result<Reading, String>) -> Option<Action> {
        match (self.phase, sample) {
            (Phase::Halted, _) => None,
            (_, Ok(r)) => self.observe(now, r),
            (_, Err(_)) => self.miss(now),
        }
    }

    fn miss(&mut self, now: Instant) -> Option<Action> {
        self.fail_streak = self.fail_streak.saturating_add(1);
        self.last = None;
        if let Some(action) = self.timeout(now) {
            return Some(action);
        }
        let lost = self.fail_streak == self.cfg.sensor_fail_samples;
        match self.phase {
            Phase::Normal | Phase::Recovering { .. } if lost => {
                self.phase = Phase::Blind;
                Some(Action::SensorLost)
            }
            _ => None,
        }
    }

    fn observe(&mut self, now: Instant, r: Reading) -> Option<Action> {
        self.fail_streak = 0;
        self.last = Some(r);
        let verdict = assess(&r, &self.cfg);
        let level = verdict.map(|v| v.0);
        let bump = |streak: u32, hit: bool| if hit { streak.saturating_add(1) } else { 0 };
        self.crit_streak = bump(self.crit_streak, level == Some(Level::Crit));
        self.warn_streak = bump(self.warn_streak, level.is_some());
        self.ok_streak = bump(self.ok_streak, calm(&r, &self.cfg));

        if let Some((_, fault)) = verdict.filter(|_| self.crit_streak >= self.cfg.crit_samples) {
            return self.halt(now, fault);
        }
        let warned = self.warn_streak >= self.cfg.warn_samples;
        let calmed = self.ok_streak >= self.cfg.warn_samples;
        let restore = Duration::from_secs(self.cfg.power_cap.restore_after_secs);
        match (self.phase, verdict) {
            (Phase::Recovering { strict: true, .. }, Some((_, fault))) if warned => self.halt(now, fault),
            (Phase::Normal | Phase::Blind | Phase::Recovering { .. }, Some((_, fault))) if warned => {
                self.phase = Phase::Warning {
                    since: now,
                    fault,
                    escalated: false,
                };
                Some(Action::Warn(fault))
            }
            (Phase::Warning { since, escalated, .. }, Some((_, fault))) => {
                self.phase = Phase::Warning {
                    since,
                    fault,
                    escalated,
                };
                self.timeout(now)
            }
            (Phase::Warning { .. }, None) if calmed => {
                self.phase = Phase::Recovering {
                    since: now,
                    strict: true,
                };
                Some(Action::Recovered)
            }
            (Phase::Blind, None) => {
                self.phase = Phase::Recovering {
                    since: now,
                    strict: false,
                };
                Some(Action::SensorBack)
            }
            (Phase::Recovering { since, .. }, None) if now.duration_since(since) >= restore => {
                self.phase = Phase::Normal;
                None
            }
            _ => self.timeout(now),
        }
    }

    fn timeout(&mut self, now: Instant) -> Option<Action> {
        match self.phase {
            Phase::Warning { since, fault, .. }
                if now.duration_since(since) >= Duration::from_secs(self.cfg.recover_secs) =>
            {
                self.halt(now, fault)
            }
            _ => None,
        }
    }

    fn halt(&mut self, now: Instant, fault: Fault) -> Option<Action> {
        if self.cfg.shutdown {
            self.phase = Phase::Halted;
            return Some(Action::Shutdown(fault));
        }
        let (since, first) = match self.phase {
            Phase::Warning { since, escalated, .. } => (since, !escalated),
            _ => (now, true),
        };
        self.phase = Phase::Warning {
            since,
            fault,
            escalated: true,
        };
        first.then_some(Action::Critical(fault))
    }
}
