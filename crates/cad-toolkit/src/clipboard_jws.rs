//! JWS conversion into a validated portable part, with the full boundary report.
use crate::{Result, ToolkitError, clipboard};
use serde::Serialize;
use std::{fs::File, io::Read, path::Path};

#[derive(Debug, Serialize)]
pub struct JwsClipboard {
    pub report: cad_import_jww::SymbolImportReport,
    pub document: Option<clipboard::ClipboardDocument>,
}

/// Conversion is read-only. Even blocked binary records return a report.
pub fn convert(data: &[u8], name: &str, coordinate_scale: f64) -> Result<JwsClipboard> {
    if data.len() > 64 * 1024 * 1024 {
        return Err(ToolkitError::Invalid(
            "JWS exceeds the 64 MiB input limit".into(),
        ));
    }
    let candidate = tempfile::tempdir().map_err(|e| ToolkitError::Invalid(e.to_string()))?;
    let mut report =
        cad_import_jww::prepare_symbol_import(data, name, coordinate_scale, candidate.path())
            .map_err(|e| ToolkitError::Invalid(e.to_string()))?;
    let document = if report.status == "converted" {
        let prepared = (|| {
            let source = cad_model::load_project(candidate.path())
                .map_err(|e| ToolkitError::Invalid(e.to_string()))?;
            let drawing = report
                .drawing_name
                .as_deref()
                .ok_or_else(|| ToolkitError::Invalid("JWS drawing is missing".into()))?;
            let ids = source
                .drawings
                .iter()
                .find(|d| d.name == drawing)
                .ok_or_else(|| ToolkitError::Invalid("JWS drawing is missing".into()))?
                .entities
                .iter()
                .map(|e| e.entity.id().as_str().to_owned())
                .collect::<Vec<_>>();
            let mut document = clipboard::capture(
                candidate.path(),
                drawing,
                &ids,
                report
                    .placement_origin_mm
                    .ok_or_else(|| ToolkitError::Invalid("JWS origin is missing".into()))?,
                clipboard::DimensionCopyPolicy::IncludeReferences,
            )?;
            document
                .warnings
                .extend(report.warnings.iter().map(|warning| {
                    format!(
                        "JWS {} ({}): {}",
                        warning.code, warning.record_type, warning.message
                    )
                }));
            document.warnings.push(format!("JWS source {} converted at {} mm per coordinate unit. Palette, fonts and sheet are replacements; exact round-trip is unavailable.", report.source_hash, coordinate_scale));
            clipboard::serialize_document(&document)?;
            Ok::<_, ToolkitError>(document)
        })();
        match prepared {
            Ok(document) => Some(document),
            Err(error) => {
                report.status = "blocked".into();
                report.blockers.push(error.to_string());
                None
            }
        }
    } else {
        None
    };
    Ok(JwsClipboard { report, document })
}

pub fn read(path: &Path, coordinate_scale: f64) -> Result<JwsClipboard> {
    let file = File::open(path).map_err(|e| ToolkitError::Invalid(e.to_string()))?;
    let mut data = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|e| ToolkitError::Invalid(e.to_string()))?;
    convert(
        &data,
        path.file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("JWS part"),
        coordinate_scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn symbol() -> Vec<u8> {
        let mut data = b"JwsData.".to_vec();
        data.extend([b'.'; 192]);
        data.extend(600u32.to_le_bytes());
        for value in [2f64, 3.] {
            data.extend(value.to_le_bytes());
        }
        for _ in 0..16 {
            data.extend(100f64.to_le_bytes());
        }
        data.extend([0; 72]);
        for value in [2f64, 3., 12., 23.] {
            data.extend(value.to_le_bytes());
        }
        let mut writer = cad_jww_codec::Writer::default();
        writer.u16(1);
        writer.u16(0xffff);
        writer.u16(600);
        writer.u16(8);
        writer.raw(b"CDataSen");
        writer.u32(0);
        writer.u8(1);
        writer.u16(2);
        for _ in 0..4 {
            writer.u16(0);
        }
        for value in [2., 3., 12., 23.] {
            writer.f64(value);
        }
        writer.u16(0);
        data.extend(writer.into_bytes());
        data
    }
    #[test]
    fn origin_and_units_are_applied_once_and_boundary_report_is_retained() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("part.jws");
        let bytes = symbol();
        std::fs::write(&path, &bytes).unwrap();
        let imported = read(&path, 100.).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(imported.report.status, "converted");
        assert!(!imported.report.exact_round_trip);
        let document = imported.document.unwrap();
        assert_eq!(document.base_point, [200., 300.]);
        assert!(matches!(
            document.entities[0],
            cad_model::Entity::Line {
                p1: [200., 300.],
                p2: [1200., 2300.],
                ..
            }
        ));
        assert!(!document.warnings.is_empty());
        clipboard::validate_document(&document).unwrap();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let plan = clipboard::plan(
            &root,
            &document,
            &clipboard::PasteRequest {
                drawing: "plan_1f".into(),
                at: [1000., 1000.],
                rotation_deg: 0.,
                scale: 1.,
            },
        )
        .unwrap();
        assert_eq!(plan.report.status, "ready");
        assert_eq!(plan.report.cad_check.status, cad_check::CheckStatus::Ok);
    }
    #[test]
    fn unsupported_bytes_invalid_units_and_empty_symbols_never_create_parts() {
        let mut empty = symbol();
        empty.truncate(452);
        empty.extend(0u16.to_le_bytes());
        let no_geometry = convert(&empty, "empty.jws", 1.).unwrap();
        assert!(no_geometry.document.is_none());
        let bad = convert(b"unknown", "bad.jws", 100.).unwrap();
        assert!(bad.document.is_none());
        assert_eq!(bad.report.status, "blocked");
        assert!(!bad.report.blockers.is_empty());
        for scale in [0., -1., f64::NAN, f64::INFINITY] {
            let bad = convert(&symbol(), "part", scale).unwrap();
            assert!(bad.document.is_none());
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.jws");
        File::create(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024 + 1)
            .unwrap();
        assert!(read(&path, 1.).is_err());
    }
}
