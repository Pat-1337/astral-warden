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
use sensor::Simulated;
use std::path::PathBuf;
use std::process::ExitCode;
use warden::Warden;

const USAGE: &str = "astral-warden [run|status|install|uninstall] [--config <path>]";

enum Command {
    Run,
    Status,
    Install,
    Uninstall,
    Print(&'static str),
}

fn parse() -> Result<(Command, PathBuf), String> {
    let mut command = Command::Run;
    let mut config = config::default_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "run" => command = Command::Run,
            "status" => command = Command::Status,
            "install" => command = Command::Install,
            "uninstall" => command = Command::Uninstall,
            "-c" | "--config" => config = args.next().ok_or("--config needs a path")?.into(),
            "-V" | "--version" => command = Command::Print(concat!("astral-warden ", env!("CARGO_PKG_VERSION"))),
            "-h" | "--help" => command = Command::Print(USAGE),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok((command, config))
}

fn open(cfg: &Config) -> Result<Device, String> {
    match &cfg.simulate {
        Some(path) => Ok(Device {
            name: format!("simulated ({})", path.display()),
            pci: None,
            sensor: Box::new(Simulated(path.clone())),
        }),
        None => platform::open_device(),
    }
}

fn status(cfg: &Config) -> Result<(), String> {
    let Device { name, pci, mut sensor } = open(cfg)?;
    let cap = PowerCap::new(pci, cfg.power_cap.percent);
    println!("device   {name}");
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

fn run(cfg: Config) -> Result<(), String> {
    let _lock = platform::single_instance()?;
    let Device { name, pci, sensor } = open(&cfg)?;
    log!("watching {name}");
    let cap = PowerCap::new(pci, cfg.power_cap.percent);
    platform::run(Warden::new(cfg.clone(), sensor, cap), &cfg)
}

fn main() -> ExitCode {
    platform::attach_console();
    let result = parse().and_then(|(command, path)| match command {
        Command::Install => platform::install(&path),
        Command::Uninstall => platform::uninstall(),
        Command::Status => Config::load(&path).and_then(|cfg| status(&cfg)),
        Command::Run => Config::load(&path).and_then(run),
        Command::Print(text) => {
            println!("{text}");
            Ok(())
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log!("{e}");
            platform::fatal(&e);
            ExitCode::FAILURE
        }
    }
}
