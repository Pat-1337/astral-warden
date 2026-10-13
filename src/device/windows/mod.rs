mod hwinfo;
mod nvapi;
mod wireview;

use crate::hardware::{Device, Gpu, It8915, inline_device, no_device};
use crate::log;
use crate::sensor::{Reading, Sensor};
use hwinfo::HwInfo;
use nvapi::{NvApi, NvI2c};
use std::path::Path;

struct Fallback {
    direct: Option<It8915<NvI2c>>,
    hwinfo: HwInfo,
}

impl Sensor for Fallback {
    fn read(&mut self) -> Result<Reading, String> {
        let direct = match self.direct.as_mut().map(Sensor::read) {
            Some(Ok(reading)) => return Ok(reading),
            Some(Err(e)) => format!("NVAPI: {e}; "),
            None => String::new(),
        };

        self.hwinfo.read().map_err(|e| format!("{direct}HWiNFO: {e}"))
    }
}

pub fn open(wireview_port: Option<&Path>) -> Result<Device, String> {
    let api = NvApi::load();
    let mut failures = Vec::new();
    let mut gpus = Vec::new();

    match &api {
        Err(e) => failures.push(e.clone()),
        Ok(api) => {
            for (gpu, pci) in api.gpus() {
                gpus.push(Gpu { pci, bus: api.bus(gpu) });
                if wireview_port.is_some() {
                    continue;
                }

                let Some(model) = pci.model() else {
                    failures.push(format!("{pci} is not a supported board"));
                    continue;
                };

                let direct = It8915::probe(NvI2c { api: api.clone(), gpu })
                    .inspect_err(|e| log!("direct sensor read failed ({e}), falling back to HWiNFO"))
                    .ok();
                let source = match direct {
                    Some(_) => "NVAPI, HWiNFO as fallback",
                    None => "HWiNFO",
                };

                return Ok(Device {
                    name: model,
                    detail: format!("[{pci}] via {source}"),
                    pci: Some(pci),
                    bus: api.bus(gpu),
                    sensor: Box::new(Fallback {
                        direct,
                        hwinfo: HwInfo::new(),
                    }),
                });
            }
        }
    }

    let port = wireview_port.map(|p| p.to_string_lossy());
    match wireview::open(port.as_deref()) {
        Some(Ok((sensor, model, via))) => Ok(inline_device(sensor, model, via, &gpus)),
        Some(Err(e)) => {
            failures.push(format!("WireView Pro II: {e}"));
            Err(no_device(&failures))
        }
        None => Err(no_device(&failures)),
    }
}
