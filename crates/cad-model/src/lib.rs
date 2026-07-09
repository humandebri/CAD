//! Core CAD source model crate.
//!
//! This crate owns the typed representation of the NDJSON/TOML source files.
//! Phase 0 only establishes the crate boundary; schema types arrive in Phase 1.

pub const CRATE_NAME: &str = "cad-model";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-model");
    }
}
