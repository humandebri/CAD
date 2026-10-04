use miette::{IntoDiagnostic, Result, miette};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    pub jww: bool,
    pub strict: bool,
    pub preview_dpi: Option<f64>,
}

pub fn export(
    project: &Path,
    revision: &str,
    requested: &[String],
    requested_pages: &[String],
    directory: &Path,
    options: ExportOptions,
) -> Result<()> {
    let ExportOptions {
        jww,
        strict,
        preview_dpi,
    } = options;
    super::review_bundle::validate_output_directory(project, directory)?;
    if preview_dpi.is_some_and(|dpi| !dpi.is_finite() || !(36.0..=300.0).contains(&dpi)) {
        return Err(miette!("preview DPI must be 36..300"));
    }
    let snapshot =
        cad_git::snapshot(project, &cad_git::Revision::parse(revision)).into_diagnostic()?;
    let source = &snapshot.source;
    let check = cad_check::check_project(&source.root);
    if !check.is_ok() {
        return Err(miette!(
            "export source failed CAD validation: {}",
            serde_json::to_string(&check).into_diagnostic()?
        ));
    }
    let selections: Vec<(String, Option<String>)> = if !requested_pages.is_empty() {
        requested_pages
            .iter()
            .map(|page| {
                let (drawing, layout) = page
                    .rsplit_once('@')
                    .ok_or_else(|| miette!("page must be DRAWING@LAYOUT"))?;
                if drawing.is_empty() || layout.is_empty() {
                    return Err(miette!("page must include drawing and layout"));
                }
                Ok((drawing.to_owned(), Some(layout.to_owned())))
            })
            .collect::<Result<_>>()?
    } else if requested.is_empty() {
        source
            .drawings
            .iter()
            .map(|drawing| (drawing.name.clone(), None))
            .collect::<Vec<_>>()
    } else {
        requested.iter().map(|name| (name.clone(), None)).collect()
    };
    let mut unique = BTreeSet::new();
    let mut checks = Vec::new();
    for (name, layout) in &selections {
        let drawing = source
            .drawings
            .iter()
            .find(|drawing| drawing.name == *name)
            .ok_or_else(|| miette!("missing drawing {name:?}"))?;
        let selected_layout = layout.as_deref().unwrap_or(&drawing.layouts.active_layout);
        if !unique.insert((name, selected_layout))
            || !drawing.layouts.layouts.contains_key(selected_layout)
        {
            return Err(miette!(
                "duplicate or missing page {name}@{selected_layout}"
            ));
        }
        if jww {
            if selected_layout != drawing.layouts.active_layout {
                return Err(miette!(
                    "JWW output-set pages currently require the drawing's active layout; use PDF/SVG for alternate layouts"
                ));
            }
            let report = cad_check::check_project_for_target(
                &source.root,
                cad_check::CheckTarget::JwwV600,
                Some(name),
            );
            if !report.is_ok() {
                return Err(miette!(
                    "JWW check failed: {}",
                    serde_json::to_string(&report).into_diagnostic()?
                ));
            }
            checks.push(json!({ "drawing": name, "report": report }));
        }
    }
    let pdf_selections = selections
        .iter()
        .map(|(name, layout)| (name.as_str(), layout.as_deref()))
        .collect::<Vec<_>>();
    let pdf = cad_render_pdf::render_drawings_pdf(source, &pdf_selections).into_diagnostic()?;
    if let Some(parent) = directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).into_diagnostic()?;
    }
    // A new directory is the publication boundary; a completed manifest is written last.
    // Failures retain reports/artifacts and never replace an existing export set.
    fs::create_dir(directory).into_diagnostic()?;
    let mut files = Vec::new();
    files.push(publish(directory, "set.pdf", &pdf)?);
    let mut pages = Vec::new();
    for (index, (name, layout)) in selections.iter().enumerate() {
        let prefix = format!("{:03}", index + 1);
        let mut page_source = source.clone();
        if let Some(layout) = layout {
            page_source
                .drawings
                .iter_mut()
                .find(|drawing| drawing.name == *name)
                .expect("validated drawing")
                .layouts
                .active_layout = layout.clone();
        }
        let svg = cad_render_svg::render_drawing_svg(&page_source, name).into_diagnostic()?;
        files.push(publish(
            directory,
            &format!("{prefix}.svg"),
            svg.as_bytes(),
        )?);
        if let Some(dpi) = preview_dpi {
            let png = crate::preview_png::render(&svg, dpi)?;
            files.push(publish(directory, &format!("{prefix}.png"), &png)?);
        }
        let drawing = page_source
            .drawings
            .iter()
            .find(|drawing| drawing.name == *name)
            .expect("validated drawing");
        pages.push(
            json!({ "page": index + 1, "drawing": name, "layout": drawing.layouts.active() }),
        );
        if jww {
            let output = directory.join(format!("{prefix}.jww"));
            let report_path = directory.join(format!("{prefix}.jww.report.json"));
            let report = cad_export_jww::export_jww_file_auto_with_report(
                &source.root,
                name,
                &output,
                &report_path,
                cad_export_jww::AutoExportOptions {
                    strict,
                    overwrite: false,
                    require_preservation: false,
                },
            )
            .into_diagnostic()?;
            files.push(describe(directory, &format!("{prefix}.jww.report.json"))?);
            if report.status == cad_export_jww::ExportStatus::Blocked {
                return Err(miette!(
                    "JWW export blocked; compatibility report retained at {}",
                    report_path.display()
                ));
            }
            files.push(describe(directory, &format!("{prefix}.jww"))?);
        }
    }
    if let Some(dpi) = preview_dpi {
        let gallery_pages = selections
            .iter()
            .map(|(name, layout)| {
                let drawing = source
                    .drawings
                    .iter()
                    .find(|d| &d.name == name)
                    .expect("validated drawing");
                (
                    name.clone(),
                    layout
                        .clone()
                        .unwrap_or_else(|| drawing.layouts.active_layout.clone()),
                )
            })
            .collect::<Vec<_>>();
        files.push(publish(
            directory,
            "index.html",
            crate::preview_png::gallery(&gallery_pages, dpi).as_bytes(),
        )?);
    }
    let manifest = json!({
        "schema_version": "cad-export-set/1", "status": "complete",
        "tool_version": env!("CARGO_PKG_VERSION"), "source": snapshot.identity,
        "pages": pages, "files": files, "cad_check": check, "jww_checks": checks,
        "visual_review": "pending",
        "png_preview": preview_dpi.map(|dpi| json!({"dpi":dpi,"source":"svg","font_policy":"bundled_mplus_fallback","pdf_visual_review":"pending"})),
    });
    publish(
        directory,
        "manifest.json",
        &serde_json::to_vec_pretty(&manifest).into_diagnostic()?,
    )?;
    println!(
        "Exported {} page(s) and revision manifest to {}",
        pages.len(),
        directory.display()
    );
    Ok(())
}

fn publish(directory: &Path, name: &str, bytes: &[u8]) -> Result<Value> {
    cad_edit::atomic_publish(&directory.join(name), bytes, false).into_diagnostic()?;
    Ok(
        json!({ "path": name, "bytes": bytes.len(), "blake3": blake3::hash(bytes).to_hex().to_string() }),
    )
}
fn describe(directory: &Path, name: &str) -> Result<Value> {
    let bytes = fs::read(directory.join(name)).into_diagnostic()?;
    Ok(
        json!({ "path": name, "bytes": bytes.len(), "blake3": blake3::hash(&bytes).to_hex().to_string() }),
    )
}

pub fn verify(directory: &Path) -> Result<()> {
    verify_inventory(directory, "cad-export-set/1")
}

pub(super) fn verify_inventory(directory: &Path, schema: &str) -> Result<()> {
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).into_diagnostic()?)
            .into_diagnostic()?;
    if manifest["schema_version"] != schema || manifest["status"] != "complete" {
        return Err(miette!("artifact set has no supported completed manifest"));
    }
    let files = manifest["files"]
        .as_array()
        .ok_or_else(|| miette!("manifest has no file inventory"))?;
    let mut names = BTreeSet::new();
    for file in files {
        let name = file["path"]
            .as_str()
            .ok_or_else(|| miette!("manifest file has no path"))?;
        if !names.insert(name)
            || Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(miette!("unsafe or duplicate manifest path {name:?}"));
        }
        let path = directory.join(name);
        if !fs::symlink_metadata(&path)
            .into_diagnostic()?
            .file_type()
            .is_file()
        {
            return Err(miette!("export artifact is not a regular file: {name}"));
        }
        if describe(directory, name)? != *file {
            return Err(miette!("export artifact does not match manifest: {name}"));
        }
    }
    if files.is_empty() {
        return Err(miette!("artifact set is empty"));
    }
    println!("Verified {} export artifact(s)", files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_drawing_layout_pages_are_ordered_and_do_not_mutate_source() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = cad_git::snapshot(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance"),
            &cad_git::Revision::Worktree,
        )
        .unwrap();
        let project = &snapshot.source.root;
        let layout = project.join("drawings/acceptance/layouts.toml");
        let original = fs::read_to_string(&layout).unwrap();
        fs::write(&layout,format!("{original}\n[layouts.detail]\nname=\"detail\"\npaper=\"A4\"\norientation=\"portrait\"\nscale=\"1/100\"\norigin=[0.0,0.0]\n")).unwrap();
        assert!(cad_check::check_project(project).is_ok());
        let before = cad_model::source_manifest(project).unwrap();
        let output = temp.path().join("set");
        export(
            project,
            "worktree",
            &[],
            &["acceptance@detail".into(), "acceptance@default".into()],
            &output,
            ExportOptions::default(),
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["pages"][0]["layout"]["name"], "detail");
        assert_eq!(manifest["pages"][1]["layout"]["name"], "default");
        let pdf = fs::read_to_string(output.join("001.svg")).unwrap();
        assert!(pdf.contains("data-paper-width-mm=\"210\""));
        assert_eq!(cad_model::source_manifest(project).unwrap(), before);
        assert!(
            export(
                project,
                "worktree",
                &[],
                &["acceptance@detail".into()],
                &temp.path().join("jww"),
                ExportOptions {
                    jww: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(!temp.path().join("jww").exists());
    }
    #[test]
    fn png_gallery_manifest_and_tamper_checks_share_one_source_revision() {
        let project =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/direct-edit-guide");
        let before = cad_model::source_manifest(&project).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("preview");
        export(
            &project,
            "worktree",
            &[],
            &[],
            &out,
            ExportOptions {
                preview_dpi: Some(72.0),
                ..Default::default()
            },
        )
        .unwrap();
        verify(&out).unwrap();
        let png = fs::read(out.join("001.png")).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let manifest: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["visual_review"], "pending");
        assert_eq!(manifest["cad_check"]["status"], "ok");
        assert_eq!(
            manifest["source"]["source_manifest"],
            serde_json::to_value(&before).unwrap()
        );
        assert!(
            fs::read_to_string(out.join("index.html"))
                .unwrap()
                .contains("001.png")
        );
        assert_eq!(cad_model::source_manifest(&project).unwrap(), before);
        fs::write(out.join("001.png"), b"tampered").unwrap();
        assert!(verify(&out).is_err());
        assert!(
            export(
                &project,
                "worktree",
                &[],
                &[],
                &temp.path().join("invalid"),
                ExportOptions {
                    preview_dpi: Some(f64::NAN),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(!temp.path().join("invalid").exists());
    }
    #[test]
    fn output_set_preserves_source_identifies_layout_and_detects_tampering() {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance");
        let before = cad_model::source_manifest(&project).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("set");
        export(
            &project,
            "worktree",
            &[],
            &[],
            &output,
            ExportOptions::default(),
        )
        .unwrap();
        verify(&output).unwrap();
        assert_eq!(before, cad_model::source_manifest(&project).unwrap());
        let manifest: Value =
            serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["pages"][0]["drawing"], "acceptance");
        assert_eq!(manifest["pages"][0]["layout"]["scale"], "1/50");
        assert!(
            export(
                &project,
                "worktree",
                &[],
                &[],
                &output,
                ExportOptions::default()
            )
            .is_err()
        );
        fs::write(output.join("001.svg"), b"tampered").unwrap();
        assert!(verify(&output).is_err());
    }
}
