//! Semantic CAD diff crate.
//!
//! This crate will compare CAD source by stable entity IDs and generate JSON/SVG diffs.
//! Phase 0 only establishes the diff boundary.

pub const CRATE_NAME: &str = "cad-diff";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-diff");
    }
}
