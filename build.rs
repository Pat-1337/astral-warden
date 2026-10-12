use std::path::PathBuf;

const ICON: &str = "assets/icon.ico";
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
const LANG_EN_US: u16 = 0x0409;
const MOVEABLE_DISCARDABLE: u16 = 0x1010;
const MOVEABLE_PURE_DISCARDABLE: u16 = 0x1030;
const MOVEABLE_PURE: u16 = 0x0030;
const NAME: &str = "Astral Warden";

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="AstralWarden" version="@VERSION@"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

enum Value<'a> {
    Empty,
    Binary(&'a [u8]),
    Text(&'a str),
}

fn utf16(text: &str) -> impl Iterator<Item = u8> + '_ {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes)
}

fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

fn block(key: &str, value: Value, children: &[Vec<u8>]) -> Vec<u8> {
    let (value_len, kind) = match value {
        Value::Empty => (0, 1),
        Value::Binary(bytes) => (bytes.len(), 0),
        Value::Text(text) => (text.encode_utf16().count() + 1, 1),
    };

    let mut out = vec![0, 0];
    out.extend((value_len as u16).to_le_bytes());
    out.extend(u16::to_le_bytes(kind));
    out.extend(utf16(key));
    pad(&mut out);

    match value {
        Value::Empty => {}
        Value::Binary(bytes) => out.extend(bytes),
        Value::Text(text) => out.extend(utf16(text)),
    }

    for child in children {
        pad(&mut out);
        out.extend(child);
    }

    let len = (out.len() as u16).to_le_bytes();
    out[..2].copy_from_slice(&len);
    out
}

fn version_info() -> Vec<u8> {
    let part = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let version = format!("{major}.{minor}.{patch}.0");

    let fixed: Vec<u8> = [
        0xFEEF_04BD,
        0x0001_0000,
        major << 16 | minor,
        patch << 16,
        major << 16 | minor,
        patch << 16,
        0x3F,
        0,
        0x0004_0004,
        1,
        0,
        0,
        0,
    ]
    .iter()
    .flat_map(|v: &u32| v.to_le_bytes())
    .collect();

    let strings = [
        ("CompanyName", "Pat-1337"),
        ("FileDescription", NAME),
        ("FileVersion", version.as_str()),
        ("InternalName", "astral-warden"),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", "astral-warden.exe"),
        ("ProductName", NAME),
        ("ProductVersion", version.as_str()),
    ]
    .map(|(key, text)| block(key, Value::Text(text), &[]));

    let table = block("040904B0", Value::Empty, &strings);
    let string_info = block("StringFileInfo", Value::Empty, &[table]);
    let translation = block("Translation", Value::Binary(&[0x09, 0x04, 0xB0, 0x04]), &[]);
    let var_info = block("VarFileInfo", Value::Empty, &[translation]);

    block("VS_VERSION_INFO", Value::Binary(&fixed), &[string_info, var_info])
}

fn entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
    for field in [data.len() as u32, 32] {
        out.extend(field.to_le_bytes());
    }
    for field in [0xFFFF, kind, 0xFFFF, id] {
        out.extend(u16::to_le_bytes(field));
    }
    out.extend(0u32.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(LANG_EN_US.to_le_bytes());
    out.extend([0u8; 8]);

    out.extend(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

fn resource(ico: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]);
    let count = u16_at(4);

    let mut out = Vec::new();
    entry(&mut out, 0, 0, 0, &[]);

    let mut group = [0u16, 1, count]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    for index in 0..count {
        let dir = 6 + usize::from(index) * 16;
        let (size, offset) = (u32_at(dir + 8) as usize, u32_at(dir + 12) as usize);

        entry(
            &mut out,
            RT_ICON,
            index + 1,
            MOVEABLE_DISCARDABLE,
            &ico[offset..offset + size],
        );

        group.extend(&ico[dir..dir + 12]);
        group.extend((index + 1).to_le_bytes());
    }

    entry(&mut out, RT_GROUP_ICON, 1, MOVEABLE_PURE_DISCARDABLE, &group);
    entry(&mut out, RT_VERSION, 1, MOVEABLE_PURE, &version_info());

    let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
    entry(
        &mut out,
        RT_MANIFEST,
        1,
        MOVEABLE_PURE_DISCARDABLE,
        MANIFEST.replace("@VERSION@", &version).as_bytes(),
    );
    out
}

fn main() {
    println!("cargo:rerun-if-changed={ICON}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let ico = std::fs::read(ICON).expect("assets/icon.ico");
    let res = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("icon.res");
    std::fs::write(&res, resource(&ico)).expect("write icon.res");

    println!("cargo:rustc-link-arg-bins={}", res.display());

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
}
