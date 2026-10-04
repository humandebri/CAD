//! Drafting generators and measurements over canonical model geometry.
//! Generators return ordinary atomic edit operations; they never write sources.
pub mod clipboard;
pub mod clipboard_jws;
pub mod drafting;
pub mod massing;
pub mod measure;
pub mod part_library;
pub mod sheet;
pub mod text;

use thiserror::Error;
#[derive(Debug, Error)]
pub enum ToolkitError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, ToolkitError>;
