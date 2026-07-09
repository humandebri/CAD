//! SVG renderer crate.
//!
//! This crate will turn the typed CAD model into the canonical SVG output.
//! Phase 0 only establishes the renderer boundary.

pub const CRATE_NAME: &str = "cad-render-svg";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-render-svg");
    }
}
