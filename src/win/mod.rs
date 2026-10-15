pub mod pipe;
pub mod registry;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn os_error() -> String {
    std::io::Error::last_os_error().to_string()
}
