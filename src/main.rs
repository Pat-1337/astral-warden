#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]

mod config;
mod dylib;
mod guard;
mod hardware;
mod log;
mod platform;
mod power;
mod sensor;
mod warden;

use config::Config;
use hardware::Device;
use power::PowerCap;
use sensor::{Sensor, Simulated};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use warden::Warden;

const USAGE: &str = "astral-warden [run|status|install|uninstall] [--config <path>] [--wait] [--background] [--watch]";
const WAIT: std::time::Duration = std::time::Duration::from_secs(30);

enum Command {
    Run,
    Status,
    Install,
    Uninstall,
    Print(&'static str),
}

struct Args {
    command: Command,
    config: PathBuf,
    explicit: bool,
    wait: bool,
    background: bool,
    watch: bool,
}

fn parse() -> Result<Args, String> {
    let mut command = Command::Run;
    let (mut config, mut explicit, mut wait, mut background, mut watch) =
        (config::default_path(), false, false, false, false);
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "run" => command = Command::Run,
            "status" => command = Command::Status,
            "install" => command = Command::Install,
            "uninstall" => command = Command::Uninstall,
            "--wait" => wait = true,
            "--background" => background = true,
            "--watch" => watch = true,
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
    })
}

fn open(cfg: &Config) -> Result<Device, String> {
    match &cfg.simulate {
        Some(path) => Ok(Device {
            name: "Simulated".into(),
            detail: path.display().to_string(),
            pci: None,
            bus: None,
            sensor: Box::new(Simulated(path.clone())),
        }),
        None => platform::open_device(),
    }
}

fn watch_loop(sensor: &mut dyn Sensor, cap: &PowerCap, poll: Duration) -> Result<(), String> {
    loop {
        let reading = match sensor.read() {
            Ok(r) => {
                let (pin, amps) = r.hottest();
                format!(
                    "{:5.1} A {:4.0} W  hottest pin {} {amps:5.2} A",
                    r.total(),
                    r.watts(),
                    pin + 1
                )
            }
            Err(e) => e,
        };

        let board = cap.watts().map_or(String::new(), |w| format!("  board {w:.0} W"));
        let memory = platform::memory().map_or(String::new(), |(working, private)| {
            format!("  memory {working:.1} MB, private {private:.1} MB")
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

    println!("device   {name} {detail}");

    if watch {
        return watch_loop(sensor.as_mut(), &cap, cfg.poll());
    }

    let reading = sensor.read()?;
    for (pin, (volts, amps)) in reading.volts.iter().zip(reading.amps).enumerate() {
        println!("pin {}    {amps:5.2} A  {volts:5.2} V", pin + 1);
    }
    println!("total    {:5.1} A  {:5.0} W", reading.total(), reading.watts());

    if reading.total() >= cfg.imbalance_min_total_amps {
        let (pin, deviation) = guard::imbalance(&reading);
        println!("balance  pin {} {:+.0} %", pin + 1, deviation * 100.0);
    }

    match (cap.watts(), cap.limit_watts()) {
        (Some(w), Some(l)) => println!("board    {w:5.0} W of {l:.0} W limit"),
        _ => println!("board    power limit n/a ({})", cap.available().err().unwrap_or("?")),
    }
    match guard::assess(&reading, cfg) {
        Some((level, fault)) => println!("verdict  {level:?}: {fault}"),
        None => println!("verdict  OK"),
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

fn run(cfg: Config, config_path: &Path, wait: bool) -> Result<(), String> {
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

    platform::run(Warden::new(name, cfg.clone(), sensor, cap), &cfg, config_path)
}

fn main() -> ExitCode {
    platform::attach_console();

    let result = parse().and_then(|args| match args.command {
        Command::Install => platform::install(&args.config, args.background),
        Command::Uninstall => platform::uninstall(args.background),
        Command::Status => Config::load(&args.config, args.explicit).and_then(|cfg| status(&cfg, args.watch)),
        Command::Run => Config::load(&args.config, args.explicit).and_then(|cfg| run(cfg, &args.config, args.wait)),
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
