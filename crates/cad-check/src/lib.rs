//! Strict CAD checker crate.
//!
//! This crate will validate parsed CAD source and emit structured diagnostics.
//! Phase 0 only establishes the checker boundary.

pub const CRATE_NAME: &str = "cad-check";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-check");
    }
}
