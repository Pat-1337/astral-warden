#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]

mod config;
mod display;
mod dylib;
mod guard;
mod hardware;
mod log;
#[cfg(all(feature = "omarchy", target_os = "linux"))]
mod omarchy;
mod platform;
mod power;
mod sensor;
#[cfg(updater)]
mod sha256;
#[cfg(updater)]
mod update;
mod warden;

use config::Config;
use hardware::Device;
use power::PowerCap;
use sensor::{Sensor, Simulated};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use warden::Warden;

#[cfg(not(all(feature = "omarchy", target_os = "linux")))]
const USAGE: &str = "astral-warden [run|status|install|uninstall|config] [--config <path>] [--wait] [--silent] [--background] [--watch]";
#[cfg(all(feature = "omarchy", target_os = "linux"))]
const USAGE: &str = "astral-warden [run|status|install|uninstall|config|omarchy <install|uninstall|watch>] [--config <path>] [--wait] [--silent] [--background] [--watch]";
const WAIT: std::time::Duration = std::time::Duration::from_secs(30);

enum Command {
    Run,
    Status,
    Install,
    Uninstall,
    Defaults,
    #[cfg(all(feature = "omarchy", target_os = "linux"))]
    Omarchy(omarchy::Action),
    Print(&'static str),
}

struct Args {
    command: Command,
    config: PathBuf,
    explicit: bool,
    wait: bool,
    background: bool,
    watch: bool,
    silent: bool,
    updated: Option<String>,
}

fn parse() -> Result<Args, String> {
    let mut command = Command::Run;
    let (mut config, mut explicit, mut wait, mut background, mut watch, mut silent) =
        (config::default_path(), false, false, false, false, false);
    let mut updated = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "run" => command = Command::Run,
            "status" => command = Command::Status,
            "install" => command = Command::Install,
            "uninstall" => command = Command::Uninstall,
            "config" => command = Command::Defaults,
            #[cfg(all(feature = "omarchy", target_os = "linux"))]
            "omarchy" => command = Command::Omarchy(omarchy::Action::parse(args.next())?),
            "--silent" => silent = true,
            "--wait" => wait = true,
            "--background" => background = true,
            "--watch" => watch = true,
            "--secret" => display::enable_hex(),
            "--updated" => updated = Some(args.next().ok_or("--updated needs a version")?),
            "-c" | "--config" => {
                let path = PathBuf::from(args.next().ok_or("--config needs a path")?);
                config = std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                explicit = true;
            }
            "-V" | "--version" => command = Command::Print(concat!("astral-warden ", env!("CARGO_PKG_VERSION"))),
            "-h" | "--help" => command = Command::Print(USAGE),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }

    Ok(Args {
        command,
        config,
        explicit,
        wait,
        background,
        watch,
        silent,
        updated,
    })
}

fn secret_name(device: &Device) -> String {
    match (device.pci, device.bus) {
        (Some(pci), Some(bus)) => format!("{pci} @ {bus:02x}:00.0"),
        (Some(pci), None) => pci.to_string(),
        _ => "0xDEADBEEF".into(),
    }
}

fn open(cfg: &Config) -> Result<Device, String> {
    let mut device = match &cfg.simulate {
        Some(path) => Device {
            name: "Simulated".into(),
            detail: path.display().to_string(),
            pci: None,
            bus: None,
            sensor: Box::new(Simulated(path.clone())),
        },
        None => platform::open_device()?,
    };

    if display::hex() {
        device.name = secret_name(&device);
    }

    Ok(device)
}

fn watch_loop(sensor: &mut dyn Sensor, cap: &PowerCap, poll: Duration) -> Result<(), String> {
    loop {
        let reading = match sensor.read() {
            Ok(r) => {
                let (pin, amps) = r.hottest();
                format!(
                    "{:>10} {:>9}  hottest {} {}",
                    display::amps(r.total(), 1),
                    display::watts(r.watts()),
                    display::pin(pin),
                    display::amps(amps, 2)
                )
            }
            Err(e) => e,
        };

        let board = cap
            .watts()
            .map_or(String::new(), |w| format!("  board {}", display::watts(w)));
        let memory = platform::memory().map_or(String::new(), |(working, private)| {
            let kib = |mb: f64| display::count((mb * 1024.0) as u64, "KiB");
            format!("  memory {}, private {}", kib(working), kib(private))
        });

        println!("{reading}{board}{memory}");
        std::thread::sleep(poll);
    }
}

fn status(cfg: &Config, watch: bool) -> Result<(), String> {
    let Device {
        name,
        detail,
        pci,
        bus,
        mut sensor,
    } = open(cfg)?;
    let cap = PowerCap::new(pci, bus, cfg.power_cap.percent, config::state_path());

    println!("{:<8} {name} {detail}", "device");

    if watch {
        return watch_loop(sensor.as_mut(), &cap, cfg.poll());
    }

    let reading = sensor.read()?;
    for (pin, (volts, amps)) in reading.volts.iter().zip(reading.amps).enumerate() {
        println!(
            "{:<8} {:>11}  {:>11}",
            display::pin(pin),
            display::amps(amps, 2),
            display::volts(*volts)
        );
    }
    println!(
        "{:<8} {:>11}  {:>11}",
        "total",
        display::amps(reading.total(), 1),
        display::watts(reading.watts())
    );

    if reading.total() >= cfg.imbalance_min_total_amps {
        let (pin, deviation) = guard::imbalance(&reading);
        println!("{:<8} {} {}", "balance", display::pin(pin), display::percent(deviation));
    }

    match (cap.watts(), cap.limit_watts()) {
        (Some(w), Some(l)) => println!("{:<8} {} of {} limit", "board", display::watts(w), display::watts(l)),
        _ => println!(
            "{:<8} power limit n/a ({})",
            "board",
            cap.available().err().unwrap_or("?")
        ),
    }

    match guard::assess(&reading, cfg) {
        Some((level, fault)) => println!("{:<8} {level:?}: {fault}", "verdict"),
        None => println!("{:<8} OK", "verdict"),
    }

    Ok(())
}

fn lock(wait: bool) -> Result<platform::Lock, String> {
    loop {
        match platform::single_instance() {
            Err(e) if wait => {
                log!("{e}; retrying in {} s", WAIT.as_secs());
                std::thread::sleep(WAIT);
            }
            other => return other,
        }
    }
}

fn run(cfg: Config, config_path: &Path, wait: bool, silent: bool, updated: Option<String>) -> Result<(), String> {
    let _lock = lock(wait)?;
    let Device {
        name,
        detail,
        pci,
        bus,
        sensor,
    } = open(&cfg)?;

    log!("watching {name} {detail}");
    let mut cap = PowerCap::new(pci, bus, cfg.power_cap.percent, config::state_path());
    cap.resume();

    let mut warden = Warden::new(name, cfg.clone(), sensor, cap);
    if let Some(old) = updated {
        log!("updated from {old}");
        warden.updated_from(old);

        #[cfg(all(feature = "omarchy", target_os = "linux"))]
        omarchy::refresh_widgets();
    }

    #[cfg(updater)]
    update::start(cfg.auto_update && cfg.simulate.is_none());

    platform::run(warden, &cfg, config_path, silent)
}

#[cfg(updater)]
use update::after_run;

#[cfg(not(updater))]
fn after_run(result: Result<(), String>, _: bool) -> Result<(), String> {
    result
}

fn main() -> ExitCode {
    platform::attach_console();

    let result = parse().and_then(|args| match args.command {
        Command::Install => platform::install(&args.config, args.background),
        Command::Uninstall => platform::uninstall(args.background),
        Command::Status => Config::load(&args.config, args.explicit).and_then(|cfg| status(&cfg, args.watch)),
        Command::Run => {
            let updated = args.updated.is_some();
            let result = Config::load(&args.config, args.explicit)
                .and_then(|cfg| run(cfg, &args.config, args.wait, args.silent, args.updated));

            after_run(result, updated)
        }
        #[cfg(all(feature = "omarchy", target_os = "linux"))]
        Command::Omarchy(action) => action.run(),
        Command::Defaults => {
            print!("{}", Config::default_text());
            Ok(())
        }
        Command::Print(text) => {
            println!("{text}");
            Ok(())
        }
    });

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::arm();
            log!("{e}");
            platform::fatal(&e);
            ExitCode::FAILURE
        }
    }
}
