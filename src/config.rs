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
    pub auto_update: bool,
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
            auto_update: true,
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

    #[cfg_attr(not(updater), allow(dead_code))]
    pub fn set_auto_update(path: &Path, enabled: bool) -> Result<(), String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default_text(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };

        let mut lines: Vec<&str> = text.lines().collect();
        let line = format!("auto_update = {enabled}");
        let top = lines
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .unwrap_or(lines.len());

        let existing = lines[..top]
            .iter()
            .position(|l| l.trim_start().starts_with("auto_update"));
        let last = lines[..top].iter().rposition(|l| !l.trim().is_empty());

        match (existing, last) {
            (Some(i), _) => lines[i] = &line,
            (None, Some(i)) => lines.insert(i + 1, &line),
            (None, None) => lines.insert(0, &line),
        }

        let updated = lines.join("\n") + "\n";
        toml::from_str::<Self>(&updated)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .validated()?;

        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(path, updated).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn poll(&self) -> Duration {
        Duration::from_millis(self.poll_ms)
    }
}

pub fn default_path() -> PathBuf {
    #[cfg(all(windows, feature = "portable"))]
    return std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("astral-warden.exe"))
        .with_file_name("astral-warden.toml");

    #[cfg(all(windows, not(feature = "portable")))]
    return std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
        .join("astral-warden")
        .join("config.toml");

    #[cfg(not(windows))]
    return PathBuf::from("/etc/astral-warden/config.toml");
}

pub fn state_path() -> PathBuf {
    #[cfg(all(windows, feature = "portable"))]
    return default_path().with_file_name("astral-warden.power-cap");
    #[cfg(all(windows, not(feature = "portable")))]
    return default_path().with_file_name("power-cap.state");
    #[cfg(target_os = "linux")]
    return PathBuf::from("/run/astral-warden.power-cap");
    #[cfg(not(any(windows, target_os = "linux")))]
    return std::env::temp_dir().join("astral-warden.power-cap");
}

pub fn note_path() -> PathBuf {
    #[cfg(all(windows, feature = "portable"))]
    return default_path().with_file_name("astral-warden.last-shutdown");
    #[cfg(all(windows, not(feature = "portable")))]
    return default_path().with_file_name("last-shutdown.txt");
    #[cfg(target_os = "linux")]
    return PathBuf::from("/var/lib/astral-warden/last-shutdown");
    #[cfg(not(any(windows, target_os = "linux")))]
    return std::env::temp_dir().join("astral-warden.last-shutdown");
}
