//! Command-line entrypoint for cadc.
//!
//! CLI entrypoint for source loading, checking, rendering, and semantic diff.

use clap::{Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use std::fs;
use std::path::{Path, PathBuf};

mod exchange;
mod export_set;
mod merge_plan;
mod preview_png;
mod review_bundle;
mod source_tools;
mod symbol;

#[derive(Debug, Parser)]
#[command(name = "cadc", version, about = "Git-native CAD source tooling")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Generate editor JSON Schemas from canonical Rust model types")]
    SourceSchema {
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Report stored coordinates, import scale evidence and paper scale without changing sources"
    )]
    SourceCoordinates {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(
        about = "Generate a checked direct-edit NDJSON candidate with stable IDs; never writes canonical sources"
    )]
    DraftSource {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Generate pinned all-drawing PDF/SVG, semantic diff, annotation and warning artifacts for review/CI"
    )]
    ReviewBundle {
        project: PathBuf,
        #[arg(long, default_value = "HEAD")]
        base: String,
        #[arg(long, default_value = "worktree")]
        head: String,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(about = "Verify the completed review bundle's artifact inventory and hashes")]
    VerifyReviewBundle { directory: PathBuf },
    #[command(
        about = "Track entity-ID changes and semantic dependencies along a pinned first-parent Git history"
    )]
    EntityHistory {
        project: PathBuf,
        #[arg(long)]
        entity: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Attribute normalized entity fields to their last first-parent commit, with incomplete-history warnings"
    )]
    EntityBlame {
        project: PathBuf,
        #[arg(long)]
        entity: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Preview selected entities and dependencies in a checked index candidate; apply by reviewed plan hash"
    )]
    GitStage {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long, required = true)]
        entity: Vec<String>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, requires = "expected_plan")]
        apply: bool,
        #[arg(long, requires = "apply")]
        expected_plan: Option<String>,
    },
    #[command(
        about = "Preview the complete staged index and commit its reviewed tree without hooks or signing"
    )]
    GitCommit {
        project: PathBuf,
        #[arg(long)]
        message_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, requires_all = ["expected_plan", "without_hooks"])]
        apply: bool,
        #[arg(long, requires = "apply")]
        expected_plan: Option<String>,
        #[arg(
            long,
            requires = "apply",
            help = "Acknowledge that this operation runs no Git hooks and creates an unsigned commit"
        )]
        without_hooks: bool,
    },
    #[command(about = "Import supported planar ASCII DXF into a new checked canonical project")]
    ImportDxf {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        unit_mm: Option<f64>,
    },
    #[command(
        about = "Import validated JWS 351/420/600 geometry into a new checked project; retain the compatibility report"
    )]
    ImportJws {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(
            long,
            help = "Model millimetres per stored coordinate unit; e.g. 100 for paper coordinates at 1/100"
        )]
        coordinate_scale: f64,
    },
    #[command(
        about = "Convert a checked JWS symbol to a portable CAD part with a compatibility report"
    )]
    CopyJws {
        input: PathBuf,
        #[arg(long)]
        coordinate_scale: f64,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    #[command(about = "List checked CAD part/JWS thumbnails in a read-only paged folder report")]
    ListParts {
        directory: PathBuf,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 12)]
        limit: usize,
        #[arg(long)]
        coordinate_scale: Option<f64>,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(about = "Export model-space R2013 DXF with an explicit compatibility report")]
    ExportDxf {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        strict: bool,
    },
    #[command(
        about = "Build and check an entity-aware three-way merge candidate without replacing sources"
    )]
    GitMergePlan {
        project: PathBuf,
        #[arg(long)]
        base: String,
        #[arg(long, default_value = "worktree")]
        ours: String,
        #[arg(long)]
        theirs: String,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Review/apply a clean field merge to existing worktree sources with Undo; does not merge branches or stage"
    )]
    GitMergeApply {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        base: String,
        #[arg(long)]
        theirs: String,
        #[arg(long, requires = "expected_plan")]
        apply: bool,
        #[arg(long, requires = "apply")]
        expected_plan: Option<String>,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(about = "Copy selected entities and canonical definitions to a portable CAD part")]
    CopyEntities {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long, required = true)]
        entity: Vec<String>,
        #[arg(long, default_value = "0,0", allow_hyphen_values = true)]
        base_point: String,
        #[arg(long,value_enum,default_value_t=ClipboardDimensionPolicy::IncludeReferences)]
        dimensions: ClipboardDimensionPolicy,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Review/paste a portable CAD part into a drawing with remapped IDs and definitions"
    )]
    PasteEntities {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        part: PathBuf,
        #[arg(long, default_value = "0,0", allow_hyphen_values = true)]
        at: String,
        #[arg(long, default_value_t = 0., allow_hyphen_values = true)]
        rotation_deg: f64,
        #[arg(long, default_value_t = 1.)]
        scale: f64,
        #[arg(long, requires = "expected_plan")]
        apply: bool,
        #[arg(long, requires = "apply")]
        expected_plan: Option<String>,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(
        about = "Measure canonical geometry with explicit units and unsupported-entity diagnostics"
    )]
    Measure {
        project: PathBuf,
        #[arg(long)]
        drawing: Option<String>,
        #[arg(long)]
        entity: Vec<String>,
        #[arg(long, value_enum, default_value_t = MeasureFormat::Json)]
        format: MeasureFormat,
        #[arg(long, value_enum, default_value_t = MeasureAreaMode::Sum)]
        area_mode: MeasureAreaMode,
        #[arg(long, default_value_t = 0.1)]
        curve_tolerance_mm: f64,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Calculate prism projections, flat-ground sun shadows or horizontal sky visibility"
    )]
    AnalyzeMassing {
        request: PathBuf,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(about = "Generate a checked drawing edit request for drafting or massing tools")]
    Generate {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        request: PathBuf,
        #[arg(long, conflicts_with = "text_edit")]
        text_replace: bool,
        #[arg(long, conflicts_with_all = ["text_replace", "text_edit"])]
        text_style: bool,
        #[arg(
            long,
            help = "Annotation batch request: replace, set_style or set_writing_mode"
        )]
        text_edit: bool,
        #[arg(
            long,
            help = "Save massing conditions/results/source revisions to a new external JSON report"
        )]
        analysis_out: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Preview or atomically apply an external drawing edit request with revision checks"
    )]
    Edit {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(
        about = "Export ordered drawing SVGs, a multi-page PDF, optional JWWs and a revision manifest"
    )]
    ExportSet {
        project: PathBuf,
        #[arg(long, default_value = "worktree")]
        revision: String,
        #[arg(long, value_name = "NAME")]
        drawing: Vec<String>,
        #[arg(long, value_name = "DRAWING@LAYOUT", conflicts_with = "drawing")]
        page: Vec<String>,
        #[arg(long, value_name = "NEW_DIRECTORY")]
        out: PathBuf,
        #[arg(long)]
        jww: bool,
        #[arg(long, requires = "jww")]
        strict: bool,
        #[arg(
            long,
            help = "Include PNG previews and a local HTML gallery from the checked SVGs"
        )]
        preview_png: bool,
        #[arg(long, default_value_t = 144.0, requires = "preview_png")]
        preview_dpi: f64,
    },
    #[command(about = "Verify exported files against an export-set manifest")]
    VerifyExportSet { directory: PathBuf },
    #[command(about = "Review or apply independently scaled, clipped views on one SVG/PDF sheet")]
    SheetViewports {
        project: PathBuf,
        #[arg(long)]
        drawing: String,
        #[arg(long)]
        layout: String,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        preview_svg: Option<PathBuf>,
        #[arg(long, requires = "expected_plan")]
        apply: bool,
        #[arg(long, requires = "apply")]
        expected_plan: Option<String>,
        #[arg(long, default_value = "-")]
        out: PathBuf,
    },
    #[command(about = "Compare a CAD project at commits, index, or worktree without checking out")]
    GitDiff {
        project: PathBuf,
        #[arg(long, default_value = "HEAD")]
        base: String,
        #[arg(long, default_value = "worktree")]
        head: String,
        #[arg(long)]
        drawing: Option<String>,
        #[arg(long, value_enum)]
        format: DiffFormat,
        #[arg(long)]
        out: PathBuf,
    },
    #[command(about = "Inspect JWW compatibility without importing")]
    InspectJww {
        #[arg(value_name = "FILE.jww")]
        input: PathBuf,
    },
    #[command(about = "Lint canonical CAD source and optional JWW v600 output compatibility")]
    Check {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: Option<String>,

        #[arg(long, value_enum, default_value_t = CheckTargetArg::Cad)]
        target: CheckTargetArg,

        #[arg(long, value_enum)]
        format: CheckFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    #[command(about = "Normalize known CAD numeric values in source files")]
    Format {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,
    },
    #[command(about = "Import a JWW file into a new CAD source project")]
    ImportJww {
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        #[arg(long, value_name = "PROJECT_DIR")]
        out: PathBuf,

        #[arg(
            long,
            help = "Flatten block references instead of preserving definitions"
        )]
        flatten: bool,
    },
    #[command(about = "Export a CAD drawing to best-effort JWW version 600")]
    ExportJww {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: String,

        #[arg(long, value_name = "FILE.jww")]
        out: PathBuf,

        #[arg(
            long,
            help = "Deprecated compatibility alias for normal best-effort output"
        )]
        allow_lossy: bool,

        #[arg(long, help = "Reject every JWW approximation or substitution")]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(
            long,
            help = "Require valid JWW provenance; provenance is otherwise detected automatically"
        )]
        preserve: bool,

        #[arg(long, value_name = "REPORT.json")]
        report: Option<PathBuf>,
    },
    #[command(about = "Extract the byte-exact original JWW from an imported project")]
    ExtractOriginalJww {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "FILE.jww")]
        out: PathBuf,

        #[arg(long)]
        force: bool,
    },
    #[command(about = "Export a CAD drawing to PDF")]
    ExportPdf {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: String,

        #[arg(long, value_name = "NAME")]
        layout: Option<String>,

        #[arg(long, value_name = "FILE.pdf")]
        out: PathBuf,

        #[arg(long)]
        force: bool,
    },
    #[command(about = "Compare CAD source projects by stable entity IDs")]
    Diff {
        #[arg(value_name = "BASE")]
        base: PathBuf,

        #[arg(value_name = "HEAD")]
        head: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: Option<String>,

        #[arg(long, value_enum)]
        format: DiffFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    #[command(about = "Render CAD source to SVG")]
    Render {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: Option<String>,

        #[arg(long, value_name = "NAME", requires = "drawing")]
        layout: Option<String>,

        #[arg(long, value_enum)]
        format: RenderFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
}

impl Command {
    fn source_projects(&self) -> Vec<&Path> {
        match self {
            Self::EntityHistory { .. }
            | Self::EntityBlame { .. }
            | Self::ReviewBundle { .. }
            | Self::Measure { .. }
            | Self::VerifyReviewBundle { .. }
            | Self::GitStage { .. }
            | Self::GitCommit { .. }
            | Self::ImportDxf { .. }
            | Self::ImportJws { .. }
            | Self::CopyJws { .. }
            | Self::ListParts { .. }
            | Self::AnalyzeMassing { .. }
            | Self::SheetViewports { .. }
            | Self::ExportDxf { .. }
            | Self::GitDiff { .. }
            | Self::GitMergePlan { .. }
            | Self::GitMergeApply { .. }
            | Self::CopyEntities { .. }
            | Self::PasteEntities { .. }
            | Self::ExportSet { .. }
            | Self::VerifyExportSet { .. }
            | Self::SourceSchema { .. }
            | Self::DraftSource { .. } => Vec::new(),
            Self::SourceCoordinates { project, .. }
            | Self::Check { project, .. }
            | Self::Generate { project, .. }
            | Self::Edit { project, .. }
            | Self::Format { project }
            | Self::ExportJww { project, .. }
            | Self::ExtractOriginalJww { project, .. }
            | Self::ExportPdf { project, .. }
            | Self::Render { project, .. } => vec![project.as_path()],
            Self::Diff { base, head, .. } => vec![base.as_path(), head.as_path()],
            Self::ImportJww { .. } | Self::InspectJww { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ClipboardDimensionPolicy {
    IncludeReferences,
    DetachExternal,
    RejectExternal,
}
impl From<ClipboardDimensionPolicy> for cad_toolkit::clipboard::DimensionCopyPolicy {
    fn from(value: ClipboardDimensionPolicy) -> Self {
        match value {
            ClipboardDimensionPolicy::IncludeReferences => Self::IncludeReferences,
            ClipboardDimensionPolicy::DetachExternal => Self::DetachExternal,
            ClipboardDimensionPolicy::RejectExternal => Self::RejectExternal,
        }
    }
}
fn clipboard_point(value: &str) -> Result<cad_model::Point> {
    let components = value.split(',').map(str::trim).collect::<Vec<_>>();
    if components.len() != 2 {
        return Err(miette!("Point must contain x,y in model millimetres"));
    }
    let point = [
        components[0].parse::<f64>().into_diagnostic()?,
        components[1].parse::<f64>().into_diagnostic()?,
    ];
    if !point.iter().all(|value| value.is_finite()) {
        return Err(miette!("Point must be finite"));
    }
    Ok(point)
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CheckFormat {
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CheckTargetArg {
    Cad,
    JwwV600,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RenderFormat {
    Svg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DiffFormat {
    Json,
    Svg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum MeasureFormat {
    Json,
    Csv,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum MeasureAreaMode {
    Sum,
    Union,
}
impl From<MeasureAreaMode> for cad_toolkit::measure::AreaMode {
    fn from(value: MeasureAreaMode) -> Self {
        match value {
            MeasureAreaMode::Sum => Self::Sum,
            MeasureAreaMode::Union => Self::Union,
        }
    }
}

fn output_text(path: &Path, text: &str) -> Result<()> {
    if path == Path::new("-") {
        println!("{text}");
        Ok(())
    } else {
        write_text_file(path, text)
    }
}

fn resolve_output_path(out: &Path) -> Result<PathBuf> {
    let absolute = if out.is_absolute() {
        out.to_path_buf()
    } else {
        std::env::current_dir().into_diagnostic()?.join(out)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        if component == std::path::Component::ParentDir {
            resolved.pop();
        } else if component != std::path::Component::CurDir {
            resolved.push(component);
        }
        if resolved.exists() {
            resolved = fs::canonicalize(&resolved).into_diagnostic()?;
        }
    }
    Ok(resolved)
}

fn validate_stage_report_path(project: &Path, out: &Path) -> Result<()> {
    if out == Path::new("-") {
        return Ok(());
    }
    cad_exchange::files::artifact_path(project, out).into_diagnostic()?;
    let resolved = resolve_output_path(out)?;
    let root = fs::canonicalize(project).into_diagnostic()?;
    if resolved.strip_prefix(&root).is_ok_and(|relative| {
        cad_model::classify_project_source_path(relative).is_some()
            || matches!(
                relative
                    .components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some("rules" | "drawings" | "blocks" | "comments" | "interop" | ".git")
            )
            || relative.components().any(|part| {
                matches!(
                    part.as_os_str().to_str(),
                    Some(".cad-history" | ".cad-recovery" | ".cad-transactions")
                )
            })
    }) || (cad_git::repository_root(&root).is_ok()
        && cad_git::metadata_directories(&root)
            .into_diagnostic()?
            .iter()
            .any(|path| resolved.starts_with(path)))
    {
        return Err(miette!(
            "Git report cannot overwrite canonical source, provenance, history, recovery or Git metadata"
        ));
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(command) = cli.command.as_ref() {
        for project in command.source_projects() {
            recover_project(project)?;
        }
    }

    match cli.command {
        Some(Command::ReviewBundle {
            project,
            base,
            head,
            out,
        }) => review_bundle::export(&project, &base, &head, &out)?,
        Some(Command::VerifyReviewBundle { directory }) => review_bundle::verify(&directory)?,
        Some(Command::EntityBlame {
            project,
            entity,
            revision,
            limit,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            let report = cad_git::blame::entity_blame(&project, &entity, &revision, limit)
                .into_diagnostic()?;
            output_text(
                &out,
                &serde_json::to_string_pretty(&report).into_diagnostic()?,
            )?;
        }
        Some(Command::EntityHistory {
            project,
            entity,
            revision,
            limit,
            out,
        }) => {
            let report = cad_git::history::entity_history(&project, &entity, &revision, limit)
                .into_diagnostic()?;
            output_text(
                &out,
                &serde_json::to_string_pretty(&report).into_diagnostic()?,
            )?;
        }
        Some(Command::GitStage {
            project,
            drawing,
            entity,
            out,
            apply,
            expected_plan,
        }) => {
            validate_stage_report_path(&project, &out)?;
            let mut plan = cad_git::stage::plan(&project, &drawing, &entity).into_diagnostic()?;
            if !apply || out != Path::new("-") {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
            }
            if apply {
                let result = cad_git::stage::apply(
                    &mut plan,
                    expected_plan
                        .as_deref()
                        .expect("clap requires reviewed hash"),
                );
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
                result.into_diagnostic()?;
            }
        }
        Some(Command::GitCommit {
            project,
            message_file,
            out,
            apply,
            expected_plan,
            without_hooks: _,
        }) => {
            validate_stage_report_path(&project, &out)?;
            let message = fs::read_to_string(message_file).into_diagnostic()?;
            let mut plan = cad_git::commit::plan(&project, &message).into_diagnostic()?;
            if !apply || out != Path::new("-") {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
            }
            if apply {
                let result = cad_git::commit::apply(
                    &mut plan,
                    expected_plan
                        .as_deref()
                        .expect("clap requires reviewed hash"),
                );
                if let Some(oid) = &plan.report.commit_oid {
                    eprintln!("Created reviewed commit {oid}");
                }
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
                result.into_diagnostic()?;
            }
        }
        Some(Command::ImportDxf {
            input,
            out,
            report,
            unit_mm,
        }) => exchange::import_dxf(&input, &out, &report, unit_mm)?,
        Some(Command::ExportDxf {
            project,
            drawing,
            out,
            report,
            strict,
        }) => exchange::export_dxf(&project, &drawing, &out, &report, strict)?,
        Some(Command::GitMergePlan {
            project,
            base,
            ours,
            theirs,
            out,
        }) => merge_plan::export(&project, &base, &ours, &theirs, &out)?,
        Some(Command::GitMergeApply {
            project,
            drawing,
            base,
            theirs,
            apply,
            expected_plan,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            let mut plan =
                cad_git::merge_apply::plan(&project, &drawing, &base, &theirs).into_diagnostic()?;
            if !apply || out != Path::new("-") {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
            }
            let result = if apply {
                let hash = expected_plan.ok_or_else(|| {
                    miette!("--apply requires --expected-plan from the reviewed preview")
                })?;
                cad_git::merge_apply::apply(&mut plan, &hash).into_diagnostic()
            } else {
                Ok(())
            };
            if apply {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
            }
            result?;
        }
        Some(Command::ListParts {
            directory,
            offset,
            limit,
            coordinate_scale,
            out,
        }) => {
            validate_stage_report_path(&directory, &out)?;
            let report =
                cad_toolkit::part_library::list(&cad_toolkit::part_library::LibraryRequest {
                    directory: directory.display().to_string(),
                    offset,
                    limit,
                    coordinate_scale,
                })
                .into_diagnostic()?;
            output_text(
                &out,
                &serde_json::to_string_pretty(&report).into_diagnostic()?,
            )?;
        }
        Some(Command::CopyJws {
            input,
            coordinate_scale,
            out,
            report,
        }) => {
            symbol::copy_part(&input, &out, &report, coordinate_scale)?;
        }
        Some(Command::CopyEntities {
            project,
            drawing,
            entity,
            base_point,
            dimensions,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            let document = cad_toolkit::clipboard::capture(
                &project,
                &drawing,
                &entity,
                clipboard_point(&base_point)?,
                dimensions.into(),
            )
            .into_diagnostic()?;
            let bytes = cad_toolkit::clipboard::serialize_document(&document).into_diagnostic()?;
            if out == Path::new("-") {
                output_text(&out, std::str::from_utf8(&bytes).into_diagnostic()?)?;
            } else {
                let path =
                    cad_exchange::files::new_artifact_path(&project, &out).into_diagnostic()?;
                cad_edit::atomic_publish(&path, &bytes, false).into_diagnostic()?;
            }
        }
        Some(Command::PasteEntities {
            project,
            drawing,
            part,
            at,
            rotation_deg,
            scale,
            apply,
            expected_plan,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            if resolve_output_path(&out)? == resolve_output_path(&part)? {
                return Err(miette!("Paste report cannot replace its input part"));
            }
            let document = cad_toolkit::clipboard::read_document(&part).into_diagnostic()?;
            let request = cad_toolkit::clipboard::PasteRequest {
                drawing,
                at: clipboard_point(&at)?,
                rotation_deg,
                scale,
            };
            let mut plan =
                cad_toolkit::clipboard::plan(&project, &document, &request).into_diagnostic()?;
            if !apply || out != Path::new("-") {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
            }
            if apply {
                let result = cad_toolkit::clipboard::apply(
                    &mut plan,
                    expected_plan
                        .as_deref()
                        .expect("clap requires reviewed hash"),
                );
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
                )?;
                result.into_diagnostic()?;
            }
        }
        Some(Command::Measure {
            project,
            drawing,
            entity,
            format,
            area_mode,
            curve_tolerance_mm,
            out,
        }) => {
            let snapshot =
                cad_git::snapshot(&project, &cad_git::Revision::Worktree).into_diagnostic()?;
            let check = cad_check::check_project(&snapshot.source.root);
            if !check.is_ok() {
                return Err(miette!(
                    "measurement source failed CAD validation: {}",
                    serde_json::to_string(&check).into_diagnostic()?
                ));
            }
            let report = cad_toolkit::measure::measure_project_with_options(
                &snapshot.source,
                drawing.as_deref(),
                &entity,
                cad_toolkit::measure::MeasurementOptions {
                    area_mode: area_mode.into(),
                    curve_tolerance_mm,
                },
            )
            .into_diagnostic()?;
            let text = match format {
                MeasureFormat::Json => serde_json::to_string_pretty(&report).into_diagnostic()?,
                MeasureFormat::Csv => report.csv(),
            };
            output_text(&out, &text)?;
        }
        Some(Command::AnalyzeMassing { request, out }) => {
            let input = fs::canonicalize(&request).into_diagnostic()?;
            if resolve_output_path(&out)? == input {
                return Err(miette!(
                    "Analysis report cannot replace its input conditions"
                ));
            }
            validate_stage_report_path(
                input
                    .parent()
                    .ok_or_else(|| miette!("Conditions need a parent directory"))?,
                &out,
            )?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(
                    fs::File::open(&input).into_diagnostic()?,
                    1024 * 1024 + 1,
                ),
                &mut bytes,
            )
            .into_diagnostic()?;
            if bytes.len() > 1024 * 1024 {
                return Err(miette!("Massing conditions exceed 1 MiB"));
            }
            let conditions = serde_json::from_slice(&bytes).into_diagnostic()?;
            let report = cad_toolkit::massing::analyze(&conditions).into_diagnostic()?;
            output_text(
                &out,
                &serde_json::to_string_pretty(&report).into_diagnostic()?,
            )?;
        }
        Some(Command::Generate {
            project,
            drawing,
            request,
            text_replace,
            text_style,
            text_edit,
            analysis_out,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            if resolve_output_path(&request)? == resolve_output_path(&out)? {
                return Err(miette!(
                    "Generated edit output cannot overwrite its input request"
                ));
            }
            let initial_files = cad_model::source_manifest(&project).into_diagnostic()?;
            let source = cad_model::load_project(&project).into_diagnostic()?;
            let bytes = fs::read(request).into_diagnostic()?;
            let generated = if text_edit {
                cad_toolkit::text::generate(
                    &source,
                    &drawing,
                    &serde_json::from_slice(&bytes).into_diagnostic()?,
                )
                .into_diagnostic()?
            } else if text_style {
                cad_toolkit::text::set_style(
                    &source,
                    &drawing,
                    &serde_json::from_slice(&bytes).into_diagnostic()?,
                )
                .into_diagnostic()?
            } else if text_replace {
                cad_toolkit::drafting::replace_text(
                    &source,
                    &drawing,
                    &serde_json::from_slice(&bytes).into_diagnostic()?,
                )
                .into_diagnostic()?
            } else {
                cad_toolkit::drafting::generate(
                    &source,
                    &drawing,
                    &serde_json::from_slice(&bytes).into_diagnostic()?,
                )
                .into_diagnostic()?
            };
            for warning in generated.warnings {
                eprintln!("warning: {warning}");
            }
            let state = cad_edit::editor_state(&source, &drawing).into_diagnostic()?;
            let mut edit = cad_edit::DrawingEditRequest {
                drawing,
                expected_revision: state.revision,
                operation: generated.operation,
            };
            let preview = cad_edit::preview_edit(&project, &edit).into_diagnostic()?;
            if preview.source_files != initial_files {
                return Err(miette!("source changed while generating the edit request"));
            }
            edit.operation = cad_edit::EditOperation::SourceChecked {
                operation: Box::new(edit.operation),
                expected_files: preview.source_files,
            };
            if let Some(path) = analysis_out {
                if path == Path::new("-")
                    || resolve_output_path(&path)? == resolve_output_path(&out)?
                {
                    return Err(miette!("Massing report needs a separate new JSON file"));
                }
                let analysis = generated
                    .analysis_report
                    .ok_or_else(|| miette!("This generator has no massing analysis report"))?;
                let path =
                    cad_exchange::files::new_artifact_path(&project, &path).into_diagnostic()?;
                let generator_request: cad_toolkit::drafting::GeneratorRequest =
                    serde_json::from_slice(&bytes).into_diagnostic()?;
                let report = serde_json::json!({"schema_version":"cad-massing-review/1","project_path":project,"drawing":edit.drawing,"drawing_revision":edit.expected_revision,"source_files":initial_files,"generator_request":generator_request,"analysis":analysis});
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).into_diagnostic()?;
                }
                cad_edit::atomic_publish(
                    &path,
                    &serde_json::to_vec_pretty(&report).into_diagnostic()?,
                    false,
                )
                .into_diagnostic()?;
            }
            output_text(
                &out,
                &serde_json::to_string_pretty(&edit).into_diagnostic()?,
            )?;
        }
        Some(Command::Edit {
            project,
            request,
            apply,
            out,
        }) => {
            let mut request: cad_edit::DrawingEditRequest =
                serde_json::from_slice(&fs::read(request).into_diagnostic()?).into_diagnostic()?;
            let preview = cad_edit::preview_edit(&project, &request).into_diagnostic()?;
            if apply {
                if !preview.dimension_impacts.is_empty() || !preview.warnings.is_empty() {
                    return Err(miette!(
                        "edit preview requires resolution: {}",
                        serde_json::to_string(&preview).into_diagnostic()?
                    ));
                }
                request.operation = cad_edit::EditOperation::SourceChecked {
                    operation: Box::new(request.operation),
                    expected_files: preview.source_files,
                };
                let result = cad_edit::apply_edit(&project, &request).into_diagnostic()?;
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&result).into_diagnostic()?,
                )?;
            } else {
                output_text(
                    &out,
                    &serde_json::to_string_pretty(&preview).into_diagnostic()?,
                )?;
            }
        }
        Some(Command::ExportSet {
            project,
            revision,
            drawing,
            page,
            out,
            jww,
            strict,
            preview_png,
            preview_dpi,
        }) => {
            export_set::export(
                &project,
                &revision,
                &drawing,
                &page,
                &out,
                export_set::ExportOptions {
                    jww,
                    strict,
                    preview_dpi: preview_png.then_some(preview_dpi),
                },
            )?;
        }
        Some(Command::SourceSchema { out }) => source_tools::schemas(&out)?,
        Some(Command::SourceCoordinates {
            project,
            drawing,
            out,
        }) => source_tools::coordinates(&project, &drawing, &out)?,
        Some(Command::DraftSource {
            project,
            drawing,
            request,
            previous,
            out,
        }) => source_tools::draft(&project, &drawing, &request, previous.as_deref(), &out)?,
        Some(Command::VerifyExportSet { directory }) => export_set::verify(&directory)?,
        Some(Command::SheetViewports {
            project,
            drawing,
            layout,
            request,
            preview_svg,
            apply,
            expected_plan,
            out,
        }) => {
            validate_stage_report_path(&project, &out)?;
            if resolve_output_path(&request)? == resolve_output_path(&out)? {
                return Err(miette!(
                    "Sheet report cannot overwrite the viewport request"
                ));
            }
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(
                    fs::File::open(&request).into_diagnostic()?,
                    1024 * 1024 + 1,
                ),
                &mut bytes,
            )
            .into_diagnostic()?;
            if bytes.len() > 1024 * 1024 {
                return Err(miette!("Viewport request exceeds 1 MiB"));
            }
            let plan = cad_toolkit::sheet::plan(
                &project,
                &drawing,
                &layout,
                serde_json::from_slice(&bytes).into_diagnostic()?,
            )
            .into_diagnostic()?;
            if let Some(path) = preview_svg {
                if resolve_output_path(&path)? == resolve_output_path(&out)? {
                    return Err(miette!("Sheet preview SVG needs a separate new path"));
                }
                let path =
                    cad_exchange::files::new_artifact_path(&project, &path).into_diagnostic()?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).into_diagnostic()?;
                }
                cad_edit::atomic_publish(&path, plan.svg.as_bytes(), false).into_diagnostic()?;
            }
            output_text(
                &out,
                &serde_json::to_string_pretty(&plan.report).into_diagnostic()?,
            )?;
            if apply {
                cad_toolkit::sheet::apply(
                    &plan,
                    expected_plan
                        .as_deref()
                        .expect("clap requires reviewed hash"),
                )
                .into_diagnostic()?;
            }
        }
        Some(Command::GitDiff {
            project,
            base,
            head,
            drawing,
            format,
            out,
        }) => {
            let base =
                cad_git::snapshot(&project, &cad_git::Revision::parse(&base)).into_diagnostic()?;
            let head =
                cad_git::snapshot(&project, &cad_git::Revision::parse(&head)).into_diagnostic()?;
            for snapshot in [&base, &head] {
                let report = cad_check::check_project(&snapshot.source.root);
                if !report.is_ok() {
                    return Err(miette!(
                        "{} snapshot failed CAD validation: {}",
                        snapshot.identity.revision,
                        serde_json::to_string(&report).into_diagnostic()?
                    ));
                }
            }
            if let Some(name) = drawing.as_deref()
                && !base
                    .source
                    .drawings
                    .iter()
                    .chain(&head.source.drawings)
                    .any(|candidate| candidate.name == name)
            {
                return Err(miette!("drawing {name:?} was not found in either revision"));
            }
            let report =
                cad_diff::diff_selected_drawing(&base.source, &head.source, drawing.as_deref());
            let output = match format {
                DiffFormat::Json => {
                    let mut value = serde_json::to_value(&report).into_diagnostic()?;
                    value["comparison"] =
                        serde_json::json!({ "base": base.identity, "head": head.identity });
                    serde_json::to_string_pretty(&value).into_diagnostic()?
                }
                DiffFormat::Svg => cad_diff::render_diff_svg(&base.source, &head.source, &report),
            };
            write_text_file(&out, &format!("{output}\n"))?;
        }
        Some(Command::InspectJww { input }) => {
            let bytes = fs::read(&input)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to read {}", input.display()))?;
            let inspection = cad_jww_codec::inspect_document(&bytes);
            println!(
                "{}",
                serde_json::to_string_pretty(&inspection).into_diagnostic()?
            );
        }
        Some(Command::Check {
            project,
            drawing,
            target,
            format: CheckFormat::Json,
            out,
        }) => {
            let target = match target {
                CheckTargetArg::Cad => cad_check::CheckTarget::Cad,
                CheckTargetArg::JwwV600 => cad_check::CheckTarget::JwwV600,
            };
            let report = cad_check::check_project_for_target(&project, target, drawing.as_deref());
            if out == Path::new("-") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).into_diagnostic()?
                );
            } else {
                write_json_report(&out, &report)?;
            }
            if !report.is_ok() {
                return Err(miette!(
                    "check failed with {} diagnostic(s)",
                    report.diagnostics.len()
                ));
            }
        }
        Some(Command::Format { project }) => {
            let files = format_project(&project).into_diagnostic()?;
            println!("formatted {files} NDJSON file(s)");
        }
        Some(Command::ImportJws {
            input,
            out,
            report,
            coordinate_scale,
        }) => {
            symbol::import(&input, &out, &report, coordinate_scale)?;
        }
        Some(Command::ImportJww {
            input,
            out,
            flatten,
        }) => {
            let options = cad_import_jww::ImportOptions {
                block_mode: if flatten {
                    cad_import_jww::BlockMode::Flatten
                } else {
                    cad_import_jww::BlockMode::Preserve
                },
            };
            let report = cad_import_jww::import_jww_file_with_options(&input, &out, options)
                .into_diagnostic()?;
            println!(
                "imported {} entity/entities from {} into {} ({} warning(s))",
                report.supported_entities,
                input.display(),
                out.display(),
                report.warnings.len()
            );
        }
        Some(Command::ExportJww {
            project,
            drawing,
            out,
            allow_lossy,
            strict,
            force,
            preserve,
            report,
        }) => {
            if let Some(report_path) = &report {
                cad_export_jww::validate_export_targets(&out, report_path).into_diagnostic()?;
            }
            if (strict || preserve) && allow_lossy {
                return Err(miette!(
                    "--strict/--preserve and --allow-lossy cannot be combined"
                ));
            }
            let options = cad_export_jww::AutoExportOptions {
                strict: strict || preserve,
                overwrite: force,
                require_preservation: preserve,
            };
            let export = if let Some(report_path) = report {
                cad_export_jww::export_jww_file_auto_with_report(
                    &project,
                    &drawing,
                    &out,
                    report_path,
                    options,
                )
            } else {
                cad_export_jww::export_jww_file_auto(&project, &drawing, &out, options)
            }
            .into_diagnostic()?;
            println!(
                "JWW export {:?}: {} record(s), {} warning(s), {} blocker(s)",
                export.status,
                export.expanded_entities,
                export.warnings.len(),
                export.blockers.len()
            );
            if export.status == cad_export_jww::ExportStatus::Blocked {
                return Err(miette!(
                    "JWW export blocked by {} compatibility issue(s)",
                    export.blockers.len()
                ));
            }
        }
        Some(Command::ExtractOriginalJww {
            project,
            out,
            force,
        }) => {
            cad_export_jww::extract_original_jww(&project, &out, force).into_diagnostic()?;
            println!("extracted original JWW to {}", out.display());
        }
        Some(Command::ExportPdf {
            project,
            drawing,
            layout,
            out,
            force,
        }) => {
            cad_render_pdf::export_drawing_pdf(
                &project,
                &drawing,
                layout.as_deref(),
                &out,
                cad_render_pdf::PdfExportOptions {
                    overwrite: force,
                    expected_files: Vec::new(),
                },
            )
            .into_diagnostic()?;
            println!("exported PDF to {}", out.display());
        }
        Some(Command::Diff {
            base,
            head,
            drawing,
            format,
            out,
        }) => {
            let base = cad_model::load_project(&base).into_diagnostic()?;
            let head = cad_model::load_project(&head).into_diagnostic()?;
            if let Some(name) = drawing.as_deref()
                && !base
                    .drawings
                    .iter()
                    .chain(&head.drawings)
                    .any(|candidate| candidate.name == name)
            {
                return Err(miette!("drawing {name:?} was not found in either project"));
            }
            let report = cad_diff::diff_selected_drawing(&base, &head, drawing.as_deref());
            match format {
                DiffFormat::Json => {
                    let json = serde_json::to_string_pretty(&report).into_diagnostic()?;
                    write_text_file(&out, &format!("{json}\n"))?;
                }
                DiffFormat::Svg => {
                    let svg = cad_diff::render_diff_svg(&base, &head, &report);
                    write_text_file(&out, &format!("{svg}\n"))?;
                }
            }
        }
        Some(Command::Render {
            project,
            drawing,
            layout,
            format: RenderFormat::Svg,
            out,
        }) => {
            let mut source = cad_model::load_project(&project).into_diagnostic()?;
            let name = drawing
                .as_deref()
                .or_else(|| source.drawings.first().map(|item| item.name.as_str()))
                .ok_or_else(|| miette!("project has no drawings"))?
                .to_owned();
            if let Some(layout) = layout {
                let target = source
                    .drawings
                    .iter_mut()
                    .find(|candidate| candidate.name == name)
                    .ok_or_else(|| miette!("drawing {name:?} was not found"))?;
                if !target.layouts.layouts.contains_key(&layout) {
                    return Err(miette!(
                        "layout {layout:?} was not found in drawing {name:?}"
                    ));
                }
                target.layouts.active_layout = layout;
            }
            let svg = cad_render_svg::render_drawing_svg(&source, &name).into_diagnostic()?;
            write_text_file(&out, &format!("{svg}\n"))?;
        }
        None => {}
    }

    Ok(())
}

fn recover_project(project: &Path) -> Result<()> {
    cad_edit::recover_source_transactions(project).map_err(|error| {
        miette!(
            "failed to recover source transactions in {}: {error}",
            project.display()
        )
    })
}

fn write_json_report(path: &Path, report: &cad_check::CheckReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(report)
        .into_diagnostic()
        .wrap_err("failed to serialize check report")?;
    write_atomic_text(path, &format!("{text}\n"))
}

fn write_text_file(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    write_atomic_text(path, text)
}

fn write_atomic_text(path: &Path, text: &str) -> Result<()> {
    cad_edit::atomic_publish(path, text.as_bytes(), true)
        .map_err(|error| miette::miette!("failed to publish {}: {error}", path.display()))
        .wrap_err_with(|| format!("failed to publish {}", path.display()))?;
    Ok(())
}

fn format_project(project: &Path) -> std::io::Result<usize> {
    let source = cad_model::load_project(project)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut files = Vec::new();
    for drawing in &source.drawings {
        files.push(
            project
                .join("drawings")
                .join(&drawing.name)
                .join("entities.ndjson"),
        );
    }
    for drawing in &source.drawings {
        let path = project
            .join("comments")
            .join(format!("{}.ndjson", drawing.name));
        if path.exists() {
            files.push(path);
        }
    }
    for path in &files {
        format_ndjson_file(path)?;
    }
    Ok(files.len())
}

fn format_ndjson_file(path: &Path) -> std::io::Result<()> {
    let original = fs::read(path)?;
    let mut output = String::new();
    let text = String::from_utf8(original.clone())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    for (index, segment) in text.split_inclusive('\n').enumerate() {
        let (line, ending) = if let Some(line) = segment.strip_suffix("\r\n") {
            (line, "\r\n")
        } else if let Some(line) = segment.strip_suffix('\n') {
            (line, "\n")
        } else {
            (segment, "")
        };
        if line.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("empty NDJSON line at {}:{}", path.display(), index + 1),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "invalid NDJSON at {}:{}: {error}",
                    path.display(),
                    index + 1
                ),
            )
        })?;
        output.push_str(
            &serde_json::to_string(&normalize_numbers(value)).map_err(std::io::Error::other)?,
        );
        output.push_str(ending);
    }
    let metadata = fs::metadata(path)?;
    let permissions = cad_edit::permissions_snapshot(&metadata.permissions());
    let expected_revision = blake3::hash(&original).to_hex().to_string();
    cad_edit::atomic_replace_with_permissions(
        path,
        output.as_bytes(),
        &expected_revision,
        &permissions,
    )
    .map_err(|error| std::io::Error::other(error.to_string()))
}

fn normalize_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => number
            .as_f64()
            .and_then(|value| serde_json::Number::from_f64((value * 1000.0).round() / 1000.0))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Number(number)),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(normalize_numbers).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                // A comment's original entity and its digest are immutable.
                .map(|(key, value)| {
                    let value = if key == "binding" {
                        value
                    } else {
                        normalize_numbers(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn text_edit_request_mode_is_explicit_and_mutually_exclusive() {
        assert!(
            super::Cli::try_parse_from([
                "cadc",
                "generate",
                "project",
                "--drawing",
                "plan",
                "--request",
                "text.json",
                "--text-edit",
                "--out",
                "edit.json"
            ])
            .is_ok()
        );
        for incompatible in ["--text-style", "--text-replace"] {
            assert!(
                super::Cli::try_parse_from([
                    "cadc",
                    "generate",
                    "project",
                    "--drawing",
                    "plan",
                    "--request",
                    "text.json",
                    "--text-edit",
                    incompatible,
                    "--out",
                    "edit.json"
                ])
                .is_err()
            );
        }
    }
    #[test]
    fn massing_command_parses_without_project_recovery() {
        let parsed = super::Cli::try_parse_from([
            "cadc",
            "analyze-massing",
            "conditions.json",
            "--out",
            "report.json",
        ])
        .unwrap();
        assert!(parsed.command.unwrap().source_projects().is_empty());
    }
    use super::validate_stage_report_path;
    use super::{CheckTargetArg, Cli, Command, format_ndjson_file, normalize_numbers};
    use clap::Parser;
    use std::fs;
    use std::path::Path;

    #[test]
    fn rejects_identical_export_and_report_paths() {
        assert!(
            cad_export_jww::validate_export_targets(
                Path::new("result.jww"),
                Path::new("result.jww")
            )
            .is_err()
        );
        assert!(
            cad_export_jww::validate_export_targets(
                Path::new("result.jww"),
                Path::new("result.json")
            )
            .is_ok()
        );
    }

    #[test]
    fn parses_pdf_export_contract() {
        let cli = Cli::try_parse_from([
            "cadc",
            "export-pdf",
            "project",
            "--drawing",
            "plan",
            "--layout",
            "print",
            "--out",
            "plan.pdf",
            "--force",
        ])
        .expect("PDF command should parse");
        let Some(Command::ExportPdf {
            project,
            drawing,
            layout,
            out,
            force,
        }) = cli.command
        else {
            panic!("expected export-pdf command");
        };
        assert_eq!(project, Path::new("project"));
        assert_eq!(drawing, "plan");
        assert_eq!(layout.as_deref(), Some("print"));
        assert_eq!(out, Path::new("plan.pdf"));
        assert!(force);
    }

    #[test]
    fn parses_jww_preservation_contracts() {
        let export = Cli::try_parse_from([
            "cadc",
            "export-jww",
            "project",
            "--drawing",
            "plan",
            "--out",
            "plan.jww",
            "--preserve",
        ])
        .expect("preserve export should parse");
        assert!(matches!(
            export.command,
            Some(Command::ExportJww { preserve: true, .. })
        ));
        let strict = Cli::try_parse_from([
            "cadc",
            "export-jww",
            "project",
            "--drawing",
            "plan",
            "--out",
            "plan.jww",
            "--strict",
        ])
        .expect("strict export should parse");
        assert!(matches!(
            strict.command,
            Some(Command::ExportJww { strict: true, .. })
        ));
        let lint = Cli::try_parse_from([
            "cadc",
            "check",
            "project",
            "--drawing",
            "plan",
            "--target",
            "jww-v600",
            "--format",
            "json",
            "--out",
            "-",
        ])
        .expect("JWW target lint should parse");
        assert!(matches!(
            lint.command,
            Some(Command::Check {
                target: CheckTargetArg::JwwV600,
                ..
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["cadc", "inspect-jww", "source.jww"])
                .expect("inspection should parse")
                .command,
            Some(Command::InspectJww { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "cadc",
                "extract-original-jww",
                "project",
                "--out",
                "original.jww",
            ])
            .expect("extraction should parse")
            .command,
            Some(Command::ExtractOriginalJww { .. })
        ));
    }

    #[test]
    fn every_source_reading_command_declares_its_recovery_roots() {
        let cases = [
            (
                vec![
                    "cadc", "check", "project", "--format", "json", "--out", "out.json",
                ],
                1,
            ),
            (vec!["cadc", "format", "project"], 1),
            (
                vec![
                    "cadc",
                    "export-jww",
                    "project",
                    "--drawing",
                    "plan",
                    "--out",
                    "out.jww",
                ],
                1,
            ),
            (
                vec![
                    "cadc",
                    "extract-original-jww",
                    "project",
                    "--out",
                    "original.jww",
                ],
                1,
            ),
            (
                vec![
                    "cadc",
                    "export-pdf",
                    "project",
                    "--drawing",
                    "plan",
                    "--out",
                    "out.pdf",
                ],
                1,
            ),
            (
                vec![
                    "cadc", "render", "project", "--format", "svg", "--out", "out.svg",
                ],
                1,
            ),
            (
                vec![
                    "cadc", "diff", "base", "head", "--format", "json", "--out", "out.json",
                ],
                2,
            ),
        ];
        for (arguments, expected_roots) in cases {
            let cli = Cli::try_parse_from(arguments).expect("command should parse");
            assert_eq!(
                cli.command.expect("command").source_projects().len(),
                expected_roots
            );
        }

        let import =
            Cli::try_parse_from(["cadc", "import-jww", "source.jww", "--out", "new-project"])
                .expect("import should parse");
        assert!(
            import
                .command
                .expect("import command")
                .source_projects()
                .is_empty()
        );
        let inspect = Cli::try_parse_from(["cadc", "inspect-jww", "source.jww"])
            .expect("inspection should parse");
        assert!(
            inspect
                .command
                .expect("inspect command")
                .source_projects()
                .is_empty()
        );
        let symbol = Cli::try_parse_from([
            "cadc",
            "import-jws",
            "source.jws",
            "--out",
            "new-project",
            "--report",
            "report.json",
            "--coordinate-scale",
            "100",
        ])
        .expect("symbol import should parse");
        assert!(symbol.command.unwrap().source_projects().is_empty());
    }

    #[test]
    fn format_normalizes_numbers_and_preserves_crlf_and_trailing_newline() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("entities.ndjson");
        fs::write(&path, b"{\"x\":1.23456,\"unknown\":\"keep\"}\r\n")
            .expect("fixture should write");
        format_ndjson_file(&path).expect("format should succeed");
        let text = fs::read_to_string(path).expect("formatted file should read");
        assert!(text.contains("1.235"));
        assert!(text.contains("keep"));
        assert!(text.ends_with("\n"));
        assert!(text.contains("\r\n"));
        assert_eq!(
            normalize_numbers(serde_json::json!({"x": 2.3456}))["x"],
            2.346
        );
    }

    #[test]
    fn clipboard_commands_require_selection_and_review_and_allow_standalone_reports() {
        assert!(
            Cli::try_parse_from([
                "cadc",
                "copy-entities",
                "project",
                "--drawing",
                "plan",
                "--out",
                "part.json"
            ])
            .is_err()
        );
        let parsed = Cli::try_parse_from([
            "cadc",
            "copy-entities",
            "project",
            "--drawing",
            "plan",
            "--entity",
            "ent_01JZ0000000000000000000000",
            "--base-point",
            "-100,200",
            "--dimensions",
            "detach-external",
            "--out",
            "part.json",
        ])
        .unwrap();
        assert!(parsed.command.unwrap().source_projects().is_empty());
        let common = [
            "cadc",
            "paste-entities",
            "project",
            "--drawing",
            "plan",
            "--part",
            "part.json",
        ];
        let mut args = common.to_vec();
        args.push("--apply");
        assert!(Cli::try_parse_from(&args).is_err());
        args.extend(["--expected-plan", "reviewed"]);
        assert!(Cli::try_parse_from(&args).is_ok());
        assert_eq!(super::clipboard_point("-100,200").unwrap(), [-100., 200.]);
        assert!(super::clipboard_point("nan,0").is_err());
        assert!(super::clipboard_point("0,1,2").is_err());
        let temp = tempfile::tempdir().unwrap();
        assert!(
            validate_stage_report_path(temp.path(), &temp.path().join("build/paste.json")).is_ok()
        );
        assert!(
            validate_stage_report_path(temp.path(), &temp.path().join("interop/jww/new.json"))
                .is_err()
        );
    }

    #[test]
    fn reviewed_merge_requires_a_hash_and_reports_cannot_replace_protected_files() {
        assert!(
            Cli::try_parse_from([
                "cadc",
                "list-parts",
                "folder",
                "--offset",
                "12",
                "--limit",
                "12",
                "--coordinate-scale",
                "100"
            ])
            .unwrap()
            .command
            .unwrap()
            .source_projects()
            .is_empty()
        );
        let jws_args = [
            "cadc",
            "copy-jws",
            "part.jws",
            "--out",
            "part.json",
            "--report",
            "report.json",
        ];
        assert!(Cli::try_parse_from(jws_args).is_err());
        let mut args = jws_args.to_vec();
        args.extend(["--coordinate-scale", "100"]);
        assert!(
            Cli::try_parse_from(args)
                .unwrap()
                .command
                .unwrap()
                .source_projects()
                .is_empty()
        );
        let common = [
            "cadc",
            "git-merge-apply",
            "project",
            "--drawing",
            "plan",
            "--base",
            "HEAD~1",
            "--theirs",
            "HEAD",
        ];
        let parsed = Cli::try_parse_from(common).unwrap();
        assert!(parsed.command.unwrap().source_projects().is_empty());
        let mut args = common.to_vec();
        args.push("--apply");
        assert!(Cli::try_parse_from(&args).is_err());
        args.extend(["--expected-plan", "reviewed"]);
        assert!(Cli::try_parse_from(&args).is_ok());
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "reports".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .arg("init")
                .output()
                .unwrap()
                .status
                .success()
        );
        for path in [
            "cad.project.toml",
            "drawings/plan/entities.ndjson",
            "interop/jww/original.jww",
            ".cad-history/entry.json",
            "build/.cad-history/index.json",
            "build/.cad-recovery/source.bin",
            "build/.cad-transactions/pending.json",
            ".git/index",
        ] {
            assert!(
                validate_stage_report_path(root, &root.join(path)).is_err(),
                "{path}"
            );
        }
        assert!(validate_stage_report_path(root, Path::new("-")).is_ok());
        assert!(validate_stage_report_path(root, &root.join("build/merge-report.json")).is_ok());
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("cad.project.toml"), "marker").unwrap();
        for path in [
            "rules/new.json",
            "drawings/new.json",
            "blocks/new.json",
            "comments/new.json",
            "interop/new.json",
            "build/.cad-history/new.json",
        ] {
            assert!(
                validate_stage_report_path(root, &other.join(path)).is_err(),
                "{path}"
            );
        }
        assert!(validate_stage_report_path(root, &other.join("build/report.json")).is_ok());
    }

    #[test]
    fn reviewed_commit_requires_message_hash_and_disclosed_policy_without_source_recovery() {
        let common = [
            "cadc",
            "git-commit",
            "project",
            "--message-file",
            "message.txt",
            "--out",
            "report.json",
        ];
        let parsed = Cli::try_parse_from(common).unwrap();
        assert!(parsed.command.unwrap().source_projects().is_empty());
        let mut incomplete = common.to_vec();
        incomplete.extend(["--apply", "--expected-plan", "hash"]);
        assert!(Cli::try_parse_from(incomplete).is_err());
        let mut complete = common.to_vec();
        complete.extend(["--apply", "--expected-plan", "hash", "--without-hooks"]);
        assert!(Cli::try_parse_from(complete).is_ok());
    }

    #[test]
    fn formatting_keeps_recorded_comment_geometry_and_hash_valid() {
        let temp = tempfile::tempdir().unwrap();
        let mut source = cad_model::load_project(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small"),
        )
        .unwrap();
        if let cad_model::Entity::Line { p2, .. } = &mut source.drawings[0].entities[0].entity {
            p2[0] = 123.456789;
        }
        let id = source.drawings[0].entities[0]
            .entity
            .id()
            .as_str()
            .to_owned();
        let drawing = source.drawings[0].name.clone();
        let binding = cad_model::bind_comment_entity(&source, &drawing, &id, None).unwrap();
        let original = serde_json::to_value(&binding).unwrap();
        let record = serde_json::json!({"drawing":drawing,"entity_ids":[id],"anchor":{"x":1.23456,"y":2.0},"binding":original});
        let path = temp.path().join("comments.ndjson");
        fs::write(&path, format!("{record}\n")).unwrap();
        format_ndjson_file(&path).unwrap();
        let formatted: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(formatted["anchor"]["x"], 1.235);
        assert_eq!(formatted["binding"], original);
        let restored = serde_json::from_value(formatted["binding"].clone()).unwrap();
        assert_eq!(
            cad_model::evaluate_comment_binding(
                &source,
                &drawing,
                &[id],
                Some(&restored),
                &binding.source_blake3
            ),
            cad_model::CommentBindingState::Current
        );
    }
}
