use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub warn: f32,
    pub crit: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerCap {
    pub enabled: bool,
    pub percent: u32,
    pub restore_after_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub poll_ms: u64,
    pub pin_amps: Limits,
    pub imbalance: Limits,
    pub imbalance_min_total_amps: f32,
    pub warn_samples: u32,
    pub crit_samples: u32,
    pub recover_secs: u64,
    pub sensor_fail_samples: u32,
    pub shutdown: bool,
    pub alarm: bool,
    pub power_cap: PowerCap,
    pub simulate: Option<PathBuf>,
}

impl Default for PowerCap {
    fn default() -> Self {
        Self {
            enabled: true,
            percent: 80,
            restore_after_secs: 120,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_ms: 500,
            pin_amps: Limits { warn: 9.5, crit: 13.0 },
            imbalance: Limits { warn: 0.3, crit: 0.6 },
            imbalance_min_total_amps: 20.0,
            warn_samples: 6,
            crit_samples: 4,
            recover_secs: 10,
            sensor_fail_samples: 6,
            shutdown: true,
            alarm: true,
            power_cap: PowerCap::default(),
            simulate: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path, required: bool) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Self>(&text)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .validated(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn default_text() -> String {
        toml::to_string_pretty(&Self::default()).unwrap_or_default()
    }

    pub fn write_default(path: &Path) -> std::io::Result<bool> {
        if path.exists() {
            return Ok(false);
        }

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        std::fs::write(path, Self::default_text()).map(|()| true)
    }

    fn validated(self) -> Result<Self, String> {
        let checks = [
            (self.poll_ms >= 50, "poll_ms must be >= 50"),
            (
                self.pin_amps.warn < self.pin_amps.crit,
                "pin_amps.warn must be < pin_amps.crit",
            ),
            (
                0.0 < self.imbalance.warn && self.imbalance.warn < self.imbalance.crit,
                "imbalance must satisfy 0 < warn < crit",
            ),
            (
                self.warn_samples > 0 && self.crit_samples > 0 && self.sensor_fail_samples > 0,
                "sample counts must be > 0",
            ),
            (
                (50..=100).contains(&self.power_cap.percent),
                "power_cap.percent must be within 50..=100",
            ),
        ];

        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, msg)) => Err((*msg).into()),
            None => Ok(self),
        }
    }

    pub fn poll(&self) -> Duration {
        Duration::from_millis(self.poll_ms)
    }
}

pub fn default_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    #[cfg(not(windows))]
    let base = PathBuf::from("/etc");

    base.join("astral-warden").join("config.toml")
}

pub fn state_path() -> PathBuf {
    #[cfg(windows)]
    return default_path().with_file_name("power-cap.state");
    #[cfg(target_os = "linux")]
    return PathBuf::from("/run/astral-warden.power-cap");
    #[cfg(not(any(windows, target_os = "linux")))]
    return std::env::temp_dir().join("astral-warden.power-cap");
}

pub fn note_path() -> PathBuf {
    #[cfg(windows)]
    return default_path().with_file_name("last-shutdown.txt");
    #[cfg(target_os = "linux")]
    return PathBuf::from("/var/lib/astral-warden/last-shutdown");
    #[cfg(not(any(windows, target_os = "linux")))]
    return std::env::temp_dir().join("astral-warden.last-shutdown");
}
