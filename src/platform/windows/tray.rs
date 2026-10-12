use crate::warden::Severity;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Crit,
}

impl Tone {
    fn rgb(self) -> [u8; 3] {
        match self {
            Self::Ok => [0x2E, 0xB8, 0x5C],
            Self::Warn => [0xF5, 0xA6, 0x23],
            Self::Crit => [0xE5, 0x39, 0x35],
        }
    }
}

const BODY: [f32; 5] = [6.0, 12.0, 52.0, 44.0, 11.0];
const LATCH: [f32; 5] = [22.0, 6.0, 20.0, 9.0, 3.0];
const PIN_SIZE: f32 = 10.0;
const PIN_RADIUS: f32 = 3.0;
const PIN_COLUMNS: [f32; 3] = [14.0, 27.0, 40.0];
const PIN_ROWS: [f32; 2] = [22.0, 37.0];

fn inside([x, y, w, h, r]: [f32; 5], px: f32, py: f32) -> bool {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let dx = ((px - x - hw).abs() - (hw - r)).max(0.0);
    let dy = ((py - y - hh).abs() - (hh - r)).max(0.0);

    (px - x - hw).abs() <= hw && (py - y - hh).abs() <= hh && dx.hypot(dy) <= r
}

fn shade(size: f32, x: f32, y: f32) -> Option<f32> {
    let (px, py) = (x * 64.0 / size, y * 64.0 / size);
    if !inside(BODY, px, py) && !inside(LATCH, px, py) {
        return None;
    }

    let pin = PIN_ROWS.iter().any(|&row| {
        PIN_COLUMNS
            .iter()
            .any(|&col| inside([col, row, PIN_SIZE, PIN_SIZE, PIN_RADIUS], px, py))
    });

    Some(if pin { 0.25 } else { 1.0 })
}

pub fn draw(tone: Tone) -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as usize;
    let [r, g, b] = tone.rgb();
    const SS: usize = 4;

    let pixels: Vec<u8> = (0..size * size)
        .flat_map(|i| {
            let (px, py) = ((i % size) as f32, (i / size) as f32);
            let (hits, light) = (0..SS * SS)
                .filter_map(|s| {
                    let (sx, sy) = ((s % SS) as f32 + 0.5, (s / SS) as f32 + 0.5);
                    shade(size as f32, px + sx / SS as f32, py + sy / SS as f32)
                })
                .fold((0.0, 0.0), |(n, sum), l| (n + 1.0, sum + l));

            let alpha = hits / (SS * SS) as f32;
            let lit = |c: u8| (f32::from(c) * light / hits.max(1.0) * alpha) as u8;
            [lit(b), lit(g), lit(r), (alpha * 255.0) as u8]
        })
        .collect();

    let mask = vec![0u8; size.div_ceil(16) * 2 * size];
    unsafe {
        let color = CreateBitmap(size as i32, size as i32, 1, 32, pixels.as_ptr().cast());
        let mono = CreateBitmap(size as i32, size as i32, 1, 1, mask.as_ptr().cast());

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mono,
            hbmColor: color,
        };

        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mono);

        icon
    }
}

fn copy_wide<const N: usize>(dst: &mut [u16; N], text: &str) {
    let mut units = text.encode_utf16().take(N - 1).chain(std::iter::repeat(0));
    dst.fill_with(|| units.next().unwrap_or(0));
}

pub struct Tray {
    data: NOTIFYICONDATAW,
    tone: Option<Tone>,
    tip: String,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, message: u32) -> Self {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uCallbackMessage = message;

        Self {
            data,
            tone: None,
            tip: String::new(),
            added: false,
        }
    }

    fn add(&mut self) {
        self.data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        self.data.Anonymous.uVersion = NOTIFYICON_VERSION_4;

        self.added =
            unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) != 0 || Shell_NotifyIconW(NIM_MODIFY, &self.data) != 0 };
        if self.added {
            unsafe { Shell_NotifyIconW(NIM_SETVERSION, &self.data) };
        }
    }

    pub fn update(&mut self, tone: Tone, tip: &str) {
        if self.added && self.tone == Some(tone) && self.tip == tip {
            return;
        }

        if self.tone != Some(tone) {
            let old = std::mem::replace(&mut self.data.hIcon, draw(tone));
            if !old.is_null() {
                unsafe { DestroyIcon(old) };
            }
        }

        self.tone = Some(tone);
        self.tip = tip.to_owned();
        copy_wide(&mut self.data.szTip, tip);

        if self.added {
            self.data.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            self.added = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) } != 0;
        } else {
            self.add();
        }
    }

    pub fn readd(&mut self) {
        if self.tone.is_some() {
            self.add();
        }
    }

    pub fn balloon(&mut self, severity: Severity, title: &str, body: &str) {
        copy_wide(&mut self.data.szInfoTitle, title);
        copy_wide(&mut self.data.szInfo, body);

        self.data.dwInfoFlags = match severity {
            Severity::Info => NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
            Severity::Warn => NIIF_WARNING,
            Severity::Crit => NIIF_ERROR,
        };

        self.data.uFlags = NIF_INFO | NIF_SHOWTIP;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) };
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data);
            if !self.data.hIcon.is_null() {
                DestroyIcon(self.data.hIcon);
            }
        }
    }
}
