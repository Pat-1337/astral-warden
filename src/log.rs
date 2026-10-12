use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1 << 20;

struct Sink {
    path: Option<PathBuf>,
    file: Option<File>,
    armed: bool,
}

static SINK: Mutex<Sink> = Mutex::new(Sink {
    path: None,
    file: None,
    armed: false,
});

fn sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn to_file(path: PathBuf) {
    sink().path = Some(path);
}

pub fn arm() -> bool {
    !std::mem::replace(&mut sink().armed, true)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn has_file() -> bool {
    sink().path.as_ref().is_some_and(|p| p.exists())
}

fn open(path: &PathBuf) -> Option<File> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    OpenOptions::new().create(true).append(true).open(path).ok()
}

pub fn write(msg: &str) {
    let line = match std::env::var_os("JOURNAL_STREAM") {
        Some(_) => format!("{msg}\n"),
        None => format!("{} {msg}\n", timestamp()),
    };

    let _ = std::io::stderr().write_all(line.as_bytes());

    let mut sink = sink();
    let Sink {
        path: Some(path),
        file,
        armed: true,
    } = &mut *sink
    else {
        return;
    };

    if file
        .as_ref()
        .is_some_and(|f| f.metadata().is_ok_and(|m| m.len() > MAX_BYTES))
    {
        let _ = std::fs::rename(&*path, path.with_extension("log.old"));
        *file = None;
    }

    if file.is_none() {
        *file = open(path);
    }

    if let Some(file) = file {
        let _ = file.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}
