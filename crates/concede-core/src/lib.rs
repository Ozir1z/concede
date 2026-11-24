//! Concede core library.
//!
//! All domain logic (decision log, rules, criticality, change store) lives here.
//! The CLI and the app are thin layers on top of this crate.

/// The product name shown in the UI and CLI.
pub const PRODUCT_NAME: &str = "Concede";

/// The version of the core library, taken from the workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A short identification string, e.g. `"Concede 0.1.0"`.
pub fn product_banner() -> String {
    format!("{PRODUCT_NAME} {VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_contains_name_and_version() {
        let banner = product_banner();
        assert!(banner.starts_with(PRODUCT_NAME));
        assert!(banner.ends_with(VERSION));
    }
}
