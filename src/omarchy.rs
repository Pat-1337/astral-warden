// Status file layout; `watch` reads its fields by position:
//   {"ts":<unix secs>,"level":"ok|warn|crit","name":"..","text":"..","rows":[["..",".."],..]}

use crate::guard::Phase;
use crate::platform::spawn_quiet;
use crate::warden::Warden;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DIR: &str = "/run/gpu-warden";
const STATUS: &str = "/run/gpu-warden/status.json";
const STAGED: &str = "/run/gpu-warden/status.json.new";
const REQUEST: &str = "/run/gpu-warden/check-update";
const PUBLISH_EVERY: Duration = Duration::from_secs(1);
const STALE_SECS: u64 = 5;
const CLEAR_SCREEN: &str = "\x1b[H\x1b[2J";

const PLUGIN_ID: &str = "pat-1337.gpu-warden";
const MANIFEST: &str = include_str!("../omarchy/manifest.json");
const WIDGET: &str = include_str!("../omarchy/BarWidget.qml");

pub enum Action {
    Install,
    Uninstall,
    Refresh,
    Update,
    Watch,
}

impl Action {
    pub fn parse(arg: Option<String>) -> Result<Self, String> {
        match arg.as_deref() {
            Some("install") => Ok(Self::Install),
            Some("uninstall") => Ok(Self::Uninstall),
            Some("refresh") => Ok(Self::Refresh),
            Some("update") => Ok(Self::Update),
            Some("watch") => Ok(Self::Watch),
            _ => Err("usage: gpu-warden omarchy [install|uninstall|update|watch]".into()),
        }
    }

    pub fn run(self) -> Result<(), String> {
        match self {
            Self::Install => install(),
            Self::Uninstall => uninstall(),
            Self::Refresh => refresh(),
            Self::Update => request_update(),
            Self::Watch => watch(),
        }
    }
}

pub struct Publisher {
    at: Option<Instant>,
    level: &'static str,
    buf: String,
    requested: Option<SystemTime>,
}

fn modified(path: &str) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Publisher {
    pub fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let _ = fs::create_dir_all(DIR);
        let _ = fs::remove_file(REQUEST);
        let _ = fs::write(REQUEST, "").and_then(|()| fs::set_permissions(REQUEST, fs::Permissions::from_mode(0o666)));

        Self {
            at: None,
            level: "",
            buf: String::with_capacity(512),
            requested: modified(REQUEST),
        }
    }

    pub fn tick(&mut self, warden: &Warden) {
        let level = level(warden.phase());
        let due = self.at.is_none_or(|at| at.elapsed() >= PUBLISH_EVERY);
        if level == self.level && !due {
            return;
        }

        self.at = Some(Instant::now());
        self.level = level;
        render(&mut self.buf, level, warden);

        let _ = fs::write(STAGED, &self.buf).and_then(|()| fs::rename(STAGED, STATUS));

        let requested = modified(REQUEST);
        if requested != self.requested {
            self.requested = requested;
            crate::update::check_now();
        }
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        let _ = fs::remove_file(STATUS);
        let _ = fs::remove_file(REQUEST);
    }
}

fn level(phase: Phase) -> &'static str {
    match phase {
        Phase::Normal => "ok",
        Phase::Warning { escalated: true, .. } | Phase::Halted => "crit",
        Phase::Warning { .. } | Phase::Recovering { .. } | Phase::Blind => "warn",
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn render(out: &mut String, level: &str, warden: &Warden) {
    out.clear();
    let _ = write!(out, r#"{{"ts":{},"level":"{level}","name":"#, now());
    quote(out, warden.name());

    out.push_str(r#","text":"#);
    match warden.hottest() {
        Some((_, amps)) => quote(out, &crate::display::amps(amps, 1)),
        None => quote(out, "--"),
    }

    out.push_str(r#","rows":["#);
    for (i, (key, value)) in warden.details().iter().enumerate() {
        out.push_str(if i == 0 { "[" } else { ",[" });
        quote(out, key);
        out.push(',');
        quote(out, value);
        out.push(']');
    }
    out.push_str("]}\n");
}

fn quote(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn strings(json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = json.chars();

    while chars.by_ref().any(|c| c == '"') {
        let mut text = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    Some(c) => text.push(c),
                    None => break,
                },
                c => text.push(c),
            }
        }
        out.push(text);
    }

    out
}

fn ts(json: &str) -> Option<u64> {
    let rest = json.split_once(r#""ts":"#)?.1;
    rest.split(',').next()?.parse().ok()
}

fn request_update() -> Result<(), String> {
    use std::io::Write as _;

    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(REQUEST)
        .and_then(|mut file| file.write_all(now().to_string().as_bytes()))
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "the service is not running; start it with `sudo gpu-warden install`".into()
            }
            _ => format!("{REQUEST}: {e}"),
        })?;

    println!("checking for updates; the result shows as a notification");
    Ok(())
}

fn watch() -> Result<(), String> {
    loop {
        let json = fs::read_to_string(STATUS).unwrap_or_default();
        let fresh = ts(&json).is_some_and(|t| now().saturating_sub(t) < STALE_SECS);

        let mut frame = String::from(CLEAR_SCREEN);
        match fresh {
            true => {
                let fields = strings(&json);
                let _ = writeln!(frame, "{}\n", fields.get(4).map_or("", String::as_str));
                for row in fields.get(8..).unwrap_or_default().chunks_exact(2) {
                    let _ = writeln!(frame, "{:<10} {}", row[0], row[1]);
                }
            }
            false => frame.push_str("gpu-warden service is not running\nstart it with: sudo gpu-warden install\n"),
        }

        print!("{frame}");
        std::thread::sleep(PUBLISH_EVERY);
    }
}

fn plugin_dir() -> Result<PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("HOME is not set")?;

    Ok(config.join("omarchy").join("plugins").join(PLUGIN_ID))
}

fn require_user() -> Result<(), String> {
    match unsafe { libc::geteuid() } {
        0 => Err("run `gpu-warden omarchy ...` as your desktop user, without sudo".into()),
        _ => Ok(()),
    }
}

fn install() -> Result<(), String> {
    require_user()?;
    let dir = plugin_dir()?;
    write_plugin(&dir)?;

    let _ = spawn_quiet("omarchy-shell", &["shell", "rescanPlugins"]);
    spawn_quiet("omarchy", &["plugin", "enable", PLUGIN_ID, "--section", "right"])
        .map_err(|e| format!("{} written, but enabling it failed: {e}", dir.display()))?;

    println!("added {PLUGIN_ID} to the bar");
    if !Path::new(STATUS).exists() {
        println!("no readings yet; start the service with `sudo gpu-warden install`");
    }

    Ok(())
}

fn write_plugin(dir: &Path) -> Result<(), String> {
    let manifest = MANIFEST.replace("@VERSION@", env!("CARGO_PKG_VERSION"));

    fs::create_dir_all(dir)
        .and_then(|()| fs::write(dir.join("manifest.json"), manifest))
        .and_then(|()| fs::write(dir.join("BarWidget.qml"), WIDGET))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn refresh() -> Result<(), String> {
    require_user()?;
    let dir = plugin_dir()?;

    match dir.is_dir() {
        true => write_plugin(&dir),
        false => Ok(()),
    }
}

fn for_widget_users(bin: &str, plugin: &str, action: &str) {
    use std::os::unix::fs::MetadataExt;

    let Ok(homes) = fs::read_dir("/home") else {
        return;
    };

    for home in homes.flatten().map(|e| e.path()) {
        let Ok(meta) = fs::metadata(home.join(".config/omarchy/plugins").join(plugin)) else {
            continue;
        };
        let Some(user) = crate::platform::user_name(meta.uid()) else {
            continue;
        };

        let home_env = format!("HOME={}", home.display());
        let runtime = format!("XDG_RUNTIME_DIR=/run/user/{}", meta.uid());
        let _ = spawn_quiet(
            "runuser",
            &["-u", &user, "--", "env", &home_env, &runtime, bin, "omarchy", action],
        );
    }
}

pub fn refresh_widgets() {
    if let Ok(exe) = std::env::current_exe() {
        for_widget_users(&exe.to_string_lossy(), PLUGIN_ID, "refresh");
    }
}

fn uninstall() -> Result<(), String> {
    require_user()?;
    let dir = plugin_dir()?;

    let _ = spawn_quiet("omarchy", &["plugin", "disable", PLUGIN_ID]);
    match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", dir.display())),
        _ => {
            println!("removed {PLUGIN_ID}");
            Ok(())
        }
    }
}

pub fn for_sudo_user(bin: &str, action: &str) {
    let (Ok(user), Ok(uid)) = (std::env::var("SUDO_USER"), std::env::var("SUDO_UID")) else {
        return;
    };

    let runtime = format!("XDG_RUNTIME_DIR=/run/user/{uid}");
    let args = ["-u", &user, "-H", "env", &runtime, bin, "omarchy", action];
    if let Err(e) = spawn_quiet("sudo", &args) {
        println!("Omarchy bar widget not updated ({e}); run `gpu-warden omarchy {action}` as {user}");
    }
}
