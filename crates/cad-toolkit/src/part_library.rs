//! Read-only, paged part folder previews. Selection verifies the displayed bytes.
use crate::{Result, ToolkitError, clipboard, clipboard_jws};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn invalid(e: impl ToString) -> ToolkitError {
    ToolkitError::Invalid(e.to_string())
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryRequest {
    pub directory: String,
    pub offset: usize,
    pub limit: usize,
    pub coordinate_scale: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct LibraryEntry {
    pub name: String,
    pub path: Option<String>,
    pub kind: String,
    pub status: String,
    pub bytes: u64,
    pub source_blake3: Option<String>,
    pub entities: usize,
    pub blocks: usize,
    pub svg: Option<String>,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
    pub jws_report: Option<cad_import_jww::SymbolImportReport>,
}
#[derive(Debug, Serialize)]
pub struct LibraryReport {
    pub schema_version: String,
    pub directory: String,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub entries: Vec<LibraryEntry>,
}
#[derive(Debug, Serialize)]
pub struct LoadedPart {
    pub document: clipboard::ClipboardDocument,
    pub jws_report: Option<cad_import_jww::SymbolImportReport>,
}
fn kind(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".cadpart.json") {
        Some("cadpart")
    } else if name.ends_with(".jws") {
        Some("jws")
    } else {
        None
    }
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(invalid)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("Part library does not follow file symlinks"));
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(invalid)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("Part exceeds the 64 MiB input limit"));
    }
    Ok(bytes)
}
fn decode(path: &Path, data: &[u8], coordinate_scale: Option<f64>) -> Result<LoadedPart> {
    match kind(path) {
        Some("cadpart") => {
            let document = serde_json::from_slice(data)?;
            clipboard::validate_document(&document)?;
            Ok(LoadedPart {
                document,
                jws_report: None,
            })
        }
        Some("jws") => {
            let imported = clipboard_jws::convert(
                data,
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("JWS part"),
                coordinate_scale
                    .ok_or_else(|| invalid("JWS parts require an explicit coordinate scale"))?,
            )?;
            let document = imported.document.ok_or_else(|| {
                invalid(format!(
                    "JWS part is blocked: {}",
                    imported.report.blockers.join("; ")
                ))
            })?;
            Ok(LoadedPart {
                document,
                jws_report: Some(imported.report),
            })
        }
        _ => Err(invalid("Choose a .cadpart.json or .jws part")),
    }
}
/// Load exactly the bytes displayed in a library page. The immutable clipboard
/// can then be used after the source folder or active project changes.
pub fn load(
    path: &Path,
    expected_blake3: &str,
    coordinate_scale: Option<f64>,
) -> Result<LoadedPart> {
    let data = bytes(path)?;
    if blake3::hash(&data).to_hex().as_str() != expected_blake3 {
        return Err(invalid(
            "Part file changed since the library preview; reload the page",
        ));
    }
    decode(path, &data, coordinate_scale)
}
pub fn list(request: &LibraryRequest) -> Result<LibraryReport> {
    if request.limit == 0 || request.limit > 24 {
        return Err(invalid("Part library page size must be 1 to 24"));
    }
    if request
        .coordinate_scale
        .is_some_and(|v| !v.is_finite() || v <= 0.)
    {
        return Err(invalid("JWS coordinate scale must be finite and positive"));
    }
    let directory = fs::canonicalize(&request.directory).map_err(invalid)?;
    if !directory.is_dir() {
        return Err(invalid("Part library needs a directory"));
    }
    let mut paths: Vec<PathBuf> = vec![];
    for (index, entry) in fs::read_dir(&directory).map_err(invalid)?.enumerate() {
        if index >= 20_000 {
            return Err(invalid(
                "Part folder exceeds the 20000-entry scan limit; choose a smaller folder",
            ));
        }
        let entry = entry.map_err(invalid)?;
        let path = entry.path();
        if kind(&path).is_some() && !entry.file_type().map_err(invalid)?.is_dir() {
            paths.push(path);
        }
    }
    paths.sort();
    let total = paths.len();
    let mut entries = vec![];
    for path in paths.iter().skip(request.offset).take(request.limit) {
        let mut entry = LibraryEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: path.to_str().map(str::to_owned),
            kind: kind(path).unwrap().into(),
            status: "blocked".into(),
            bytes: 0,
            source_blake3: None,
            entities: 0,
            blocks: 0,
            svg: None,
            warnings: vec![],
            blockers: vec![],
            jws_report: None,
        };
        let prepared = (|| {
            let data = bytes(path)?;
            entry.bytes = data.len() as u64;
            entry.source_blake3 = Some(blake3::hash(&data).to_hex().to_string());
            if entry.kind == "jws" && request.coordinate_scale.is_none() {
                entry.status = "scale_required".into();
                entry
                    .blockers
                    .push("Enter JWS coordinate scale and reload this page".into());
                return Ok(());
            }
            let loaded = if entry.kind == "jws" {
                let imported =
                    clipboard_jws::convert(&data, &entry.name, request.coordinate_scale.unwrap())?;
                entry.blockers = imported.report.blockers.clone();
                entry.jws_report = Some(imported.report);
                let Some(document) = imported.document else {
                    return Ok(());
                };
                LoadedPart {
                    document,
                    jws_report: None,
                }
            } else {
                decode(path, &data, request.coordinate_scale)?
            };
            entry.entities = loaded.document.entities.len();
            entry.blocks = loaded.document.blocks.len();
            entry.warnings = loaded.document.warnings.clone();
            match clipboard::preview_document(&loaded.document) {
                Ok(svg) => entry.svg = Some(svg),
                Err(error) => entry
                    .warnings
                    .push(format!("Thumbnail unavailable: {error}")),
            }
            entry.status = "ready".into();
            Ok::<_, ToolkitError>(())
        })();
        if let Err(error) = prepared {
            entry.blockers.push(error.to_string());
        }
        entries.push(entry);
    }
    Ok(LibraryReport {
        schema_version: "cad-part-library/1".into(),
        directory: directory.to_string_lossy().into_owned(),
        total,
        offset: request.offset,
        limit: request.limit,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_sorted_read_only_and_selection_checks_file_hashes() {
        let temp = tempfile::tempdir().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance");
        let before = cad_model::source_manifest(&root).unwrap();
        let doc = clipboard::capture(
            &root,
            "acceptance",
            &["ent_01JZ0000000000000000000120".into()],
            [0., 0.],
            clipboard::DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        let good = temp.path().join("a.cadpart.json");
        fs::write(&good, clipboard::serialize_document(&doc).unwrap()).unwrap();
        fs::write(temp.path().join("b.cadpart.json"), b"invalid").unwrap();
        fs::write(temp.path().join("c.jws"), b"unknown").unwrap();
        fs::write(temp.path().join("ignore.json"), b"ignored").unwrap();
        let mut request = LibraryRequest {
            directory: temp.path().display().to_string(),
            offset: 0,
            limit: 2,
            coordinate_scale: None,
        };
        let page = list(&request).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.entries[0].status, "ready");
        assert_eq!(page.entries[1].status, "blocked");
        let svg = page.entries[0].svg.as_ref().unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("viewBox=\""));
        let hash = page.entries[0].source_blake3.as_ref().unwrap();
        let loaded = load(&good, hash, None).unwrap();
        assert_eq!(loaded.document.entities.len(), 1);
        assert!(loaded.jws_report.is_none());
        fs::write(&good, b"changed").unwrap();
        assert!(
            load(&good, hash, None)
                .unwrap_err()
                .to_string()
                .contains("changed since")
        );
        request.offset = 2;
        let page = list(&request).unwrap();
        assert_eq!(page.entries[0].status, "scale_required");
        request.coordinate_scale = Some(100.);
        let page = list(&request).unwrap();
        assert_eq!(page.entries[0].status, "blocked");
        assert!(
            !page.entries[0]
                .jws_report
                .as_ref()
                .unwrap()
                .blockers
                .is_empty()
        );
        assert_eq!(cad_model::source_manifest(&root).unwrap(), before);
        for limit in [0, 25] {
            request.limit = limit;
            assert!(list(&request).is_err());
        }
        request.limit = 2;
        request.coordinate_scale = Some(f64::NAN);
        assert!(list(&request).is_err());
    }
    #[test]
    fn large_expansions_keep_valid_parts_selectable_without_a_thumbnail() {
        let temp = tempfile::tempdir().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance");
        let mut doc = clipboard::capture(
            &root,
            "acceptance",
            &["ent_01JZ0000000000000000000120".into()],
            [0., 0.],
            clipboard::DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        let entity = serde_json::to_value(&doc.entities[0]).unwrap();
        doc.entities = (1..6001)
            .map(|i| {
                let mut value = entity.clone();
                value["id"] = format!("ent_{i:026}").into();
                serde_json::from_value(value).unwrap()
            })
            .collect();
        let path = temp.path().join("many.cadpart.json");
        fs::write(&path, clipboard::serialize_document(&doc).unwrap()).unwrap();
        let page = list(&LibraryRequest {
            directory: temp.path().display().to_string(),
            offset: 0,
            limit: 1,
            coordinate_scale: None,
        })
        .unwrap();
        let entry = &page.entries[0];
        assert_eq!(entry.status, "ready");
        assert!(entry.svg.is_none());
        assert!(entry.warnings.iter().any(|w| w.contains("budget")));
        assert_eq!(
            load(&path, entry.source_blake3.as_ref().unwrap(), None)
                .unwrap()
                .document
                .entities
                .len(),
            6000
        );
    }
    #[test]
    fn symlinks_and_oversized_files_are_visible_but_never_loaded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.cadpart.json");
        fs::File::create(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024 + 1)
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&path, temp.path().join("alias.cadpart.json")).unwrap();
        let page = list(&LibraryRequest {
            directory: temp.path().display().to_string(),
            offset: 0,
            limit: 24,
            coordinate_scale: None,
        })
        .unwrap();
        assert!(
            page.entries.iter().all(|e| e.status == "blocked"
                && e.source_blake3.is_none()
                && !e.blockers.is_empty())
        );
    }
}
