use crate::dylib::Library;
use crate::hardware::{Bus, IT8915_ADDR, PciId};
use std::ffi::c_void;
use std::sync::Arc;

type Handle = *mut c_void;
type Status = i32;
type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;

const MAX_GPUS: usize = 64;
const INITIALIZE: u32 = 0x0150_E828;
const ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const GPU_GET_PCI_IDENTIFIERS: u32 = 0x2DDF_B66E;
const I2C_READ_EX: u32 = 0x4D7B_0709;
const GPU_GET_BUS_ID: u32 = 0x1BE0_B8E5;
const I2C_INFO_V3: u32 = 3 << 16 | size_of::<I2cInfo>() as u32;
const I2C_SPEED_DEPRECATED: u32 = 0xFFFF;
const I2C_SPEED_100KHZ: u32 = 4;
const SENSOR_PORT: u8 = 1;

#[repr(C)]
struct I2cInfo {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    dev_address: u8,
    reg_address: *mut u8,
    reg_address_size: u32,
    data: *mut u8,
    size: u32,
    speed: u32,
    speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

const _: () = assert!(size_of::<I2cInfo>() == 64);

unsafe fn resolve<T: Copy>(query: QueryInterface, id: u32) -> Result<T, String> {
    let ptr = unsafe { query(id) };
    match ptr.is_null() {
        true => Err(format!("NVAPI function {id:#010x} missing")),
        false => Ok(unsafe { std::mem::transmute_copy::<*const c_void, T>(&ptr) }),
    }
}

pub struct NvApi {
    _lib: Library,
    enum_gpus: unsafe extern "C" fn(*mut [Handle; MAX_GPUS], *mut u32) -> Status,
    pci_ids: unsafe extern "C" fn(Handle, *mut u32, *mut u32, *mut u32, *mut u32) -> Status,
    i2c_read: unsafe extern "C" fn(Handle, *mut I2cInfo, *mut u32) -> Status,
    bus_id: Option<unsafe extern "C" fn(Handle, *mut u32) -> Status>,
}

unsafe impl Send for NvApi {}
unsafe impl Sync for NvApi {}

impl NvApi {
    pub fn load() -> Result<Arc<Self>, String> {
        let lib = Library::open(&["nvapi64.dll"]).ok_or("nvapi64.dll not found (NVIDIA driver missing)")?;

        unsafe {
            let query: QueryInterface = lib
                .symbol("nvapi_QueryInterface")
                .ok_or("nvapi_QueryInterface missing")?;
            let initialize: unsafe extern "C" fn() -> Status = resolve(query, INITIALIZE)?;

            let api = Self {
                enum_gpus: resolve(query, ENUM_PHYSICAL_GPUS)?,
                pci_ids: resolve(query, GPU_GET_PCI_IDENTIFIERS)?,
                i2c_read: resolve(query, I2C_READ_EX)?,
                bus_id: resolve(query, GPU_GET_BUS_ID).ok(),
                _lib: lib,
            };

            match initialize() {
                0 => Ok(Arc::new(api)),
                code => Err(format!("NvAPI_Initialize failed ({code})")),
            }
        }
    }

    pub fn bus(&self, gpu: Handle) -> Option<u32> {
        let mut bus = 0;
        (unsafe { (self.bus_id?)(gpu, &mut bus) } == 0).then_some(bus)
    }

    pub fn gpus(&self) -> Vec<(Handle, PciId)> {
        let mut handles = [std::ptr::null_mut(); MAX_GPUS];
        let mut count = 0;

        if unsafe { (self.enum_gpus)(&mut handles, &mut count) } != 0 {
            return Vec::new();
        }

        handles[..(count as usize).min(MAX_GPUS)]
            .iter()
            .filter_map(|&gpu| {
                let (mut device, mut subsystem, mut revision, mut ext) = (0, 0, 0, 0);
                let ok = unsafe { (self.pci_ids)(gpu, &mut device, &mut subsystem, &mut revision, &mut ext) } == 0;
                ok.then(|| (gpu, PciId::from_dwords(device, subsystem)))
            })
            .collect()
    }
}

pub struct NvI2c {
    pub api: Arc<NvApi>,
    pub gpu: Handle,
}

impl Bus for NvI2c {
    fn read(&mut self, reg: u8, buf: &mut [u8]) -> Result<(), String> {
        let mut reg = reg;
        let mut info = I2cInfo {
            version: I2C_INFO_V3,
            display_mask: 0,
            is_ddc_port: 0,
            dev_address: IT8915_ADDR << 1,
            reg_address: &mut reg,
            reg_address_size: 1,
            data: buf.as_mut_ptr(),
            size: buf.len() as u32,
            speed: I2C_SPEED_DEPRECATED,
            speed_khz: I2C_SPEED_100KHZ,
            port_id: SENSOR_PORT,
            is_port_id_set: 1,
        };
        let mut unknown = 0;

        match unsafe { (self.api.i2c_read)(self.gpu, &mut info, &mut unknown) } {
            0 => Ok(()),
            code => Err(format!("NvAPI_I2CReadEx failed ({code})")),
        }
    }
}
