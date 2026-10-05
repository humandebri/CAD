use cad_plan_audit::{Ledger, SCHEMA, Status};
use clap::Parser;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "cad-audit",
    about = "Read-only architectural audit outside the CAD core; explicit ID ledger required"
)]
struct Args {
    project: PathBuf,
    #[arg(long)]
    ledger: PathBuf,
    #[arg(long, help = "New output directory outside canonical CAD projects")]
    out: PathBuf,
    #[arg(
        long,
        help = "Generate checked edit requests and ledger candidates, never apply them"
    )]
    proposals: bool,
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn write_json(path: &Path, v: &impl serde::Serialize) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn run(args: Args) -> Result<Status, Box<dyn std::error::Error>> {
    let project_root = args.project.canonicalize()?;
    let parent = args
        .out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let out = parent.join(
        args.out
            .file_name()
            .ok_or("output requires directory name")?,
    );
    if out.exists()
        || parent.starts_with(&project_root)
        || parent
            .ancestors()
            .any(|p| p.join("cad.project.toml").is_file())
        || parent
            .ancestors()
            .any(|p| p.file_name().is_some_and(|n| n == ".git"))
    {
        return Err(
            "output must be a new external directory, outside canonical projects and Git metadata"
                .into(),
        );
    }
    let before = cad_model::source_manifest(&project_root)?;
    let ledger_bytes = fs::read(&args.ledger)?;
    let ledger: Ledger = serde_json::from_slice(&ledger_bytes)?;
    let source = cad_model::load_project(&project_root)?;
    let check = cad_check::check_loaded_project(&source);
    if check.status == cad_check::CheckStatus::Error {
        return Err(format!("CAD source check failed: {:?}", check.diagnostics).into());
    }
    let analysis = cad_plan_audit::analyze(&source, &ledger)?;
    let mut artifacts = Vec::<(String, Vec<u8>)>::new();
    let mut repair_results = vec![];
    if args.proposals {
        for (i, repair) in analysis.repairs.iter().enumerate() {
            if !repair.available {
                repair_results.push(json!({"repair":repair,"candidate_status":"unavailable"}));
                continue;
            }
            let drawing_file = project_root
                .join("drawings")
                .join(&repair.drawing)
                .join("entities.ndjson");
            let revision = blake3::hash(&fs::read(drawing_file)?).to_hex().to_string();
            match cad_plan_audit::repair_candidate(&source,&ledger,repair,&revision,before.clone()) {
                Ok(candidate)=>{
                    let prefix=format!("repair-{:03}",i+1);
                    let preview=cad_edit::preview_edit(&project_root,&candidate.request)?;
                    let after=cad_plan_audit::analyze(&candidate.project,&candidate.ledger)?;
                    artifacts.push((format!("{prefix}.edit.json"),serde_json::to_vec_pretty(&candidate.request)?));
                    artifacts.push((format!("{prefix}.ledger.json"),serde_json::to_vec_pretty(&candidate.ledger)?));
                    artifacts.push((format!("{prefix}.preview.json"),serde_json::to_vec_pretty(&preview)?));
                    artifacts.push((format!("{prefix}.after-report.json"),serde_json::to_vec_pretty(&after.report)?));
                    artifacts.push((format!("{prefix}.after.svg"),cad_render_svg::render_drawing_svg(&candidate.project,&repair.drawing)?.into_bytes()));
                    repair_results.push(json!({"repair":repair,"candidate_status":"generated","prefix":prefix,"requires_review":true,"warnings":preview.warnings,"dimension_impacts":preview.dimension_impacts}));
                },Err(error)=>repair_results.push(json!({"repair":repair,"candidate_status":"blocked","reason":error.to_string()})),
            }
        }
    }
    let mut html = String::from(
        "<!doctype html><html lang=\"ja\"><meta charset=\"utf-8\"><title>図面検査</title><style>body{font:16px sans-serif;margin:30px;max-width:1300px;color:#24313a}table{border-collapse:collapse;width:100%;margin:25px 0}td,th{border:1px solid #ccd5dc;padding:8px;text-align:left}img{width:100%;border:1px solid #ccd5dc}.fail{background:#fee}.unknown{background:#fff6da}</style><h1>図面検査</h1>",
    );
    html.push_str(&format!("<p>状態：{:?}。対応台帳に登録した対象のみ検査。資料不足はUnknown。CAD正本は変更していません。</p><p><a href=\"report.json\">検査JSON</a> / <a href=\"schedule.json\">建具・設備表</a> / <a href=\"elevations.svg\">建具寸法模式図</a> / <a href=\"repairs.json\">修正候補</a></p>",analysis.report.status));
    html.push_str("<table><tr><th>状態</th><th>図面・対象</th><th>検査</th><th>値 / 条件</th><th>説明</th></tr>");
    for c in &analysis.report.checks {
        html.push_str(&format!("<tr class=\"{}\"><td>{:?}</td><td>{} / {}</td><td>{}</td><td>{:?} / {:?}</td><td>{}</td></tr>",match c.status{Status::Pass=>"pass",Status::Fail=>"fail",Status::Unknown=>"unknown"},c.status,escape(&c.drawing),escape(&c.object),escape(&c.code),c.measured,c.required,escape(&c.message)));
    }
    html.push_str("</table><h2>対象図面と検出箇所</h2>");
    let mut schedules = vec![];
    let mut elevations = vec![];
    for (i, d) in ledger.drawings.iter().enumerate() {
        let mut svg = cad_render_svg::render_drawing_svg(&source, &d.drawing)?;
        let mut markers = String::new();
        for (j, c) in analysis
            .report
            .checks
            .iter()
            .filter(|c| c.drawing == d.drawing && c.status != Status::Pass)
            .enumerate()
        {
            let [x, y] = c.position;
            markers.push_str(&format!("<g><title>{}</title><circle cx=\"{x}\" cy=\"{}\" r=\"100\" fill=\"none\" stroke=\"#d54b25\" stroke-width=\"12\"/><text x=\"{x}\" y=\"{}\" font-size=\"100\" fill=\"#d54b25\">{}</text></g>",escape(&format!("{} {}",c.object,c.code)),-y,-y,j+1));
        }
        svg = svg.replace("</svg>", &format!("{markers}</svg>"));
        let name = format!("drawing-{:03}.svg", i + 1);
        artifacts.push((name.clone(), svg.into_bytes()));
        html.push_str(&format!(
            "<h3>{}</h3><img src=\"{name}\" alt=\"検査箇所\">",
            escape(&d.drawing)
        ));
        for door in &d.doors {
            let e = source
                .drawings
                .iter()
                .find(|s| s.name == d.drawing)
                .unwrap()
                .entities
                .iter()
                .find(|r| r.entity.id().as_str() == door.swing_entity_id)
                .unwrap();
            let cad_model::Entity::Arc { radius, .. } = e.entity else {
                unreachable!()
            };
            schedules.push(json!({"drawing":d.drawing,"key":door.key,"kind":"door","opening":door.opening,"leaf_width_mm":radius,"product":door.product,"source_entity_ids":[door.swing_entity_id],"height_source":"explicit ledger product dimension, never inferred from plan"}));
            if let Some(p) = &door.product
                && let Some(h) = p.nominal_height_mm
            {
                let w = p.nominal_width_mm.unwrap_or(radius);
                elevations.push((door.key.clone(), w, h, p.model.clone()));
            }
        }
        for f in &d.fixtures {
            let bounds = cad_plan_audit::fixture_bounds(&source, &d.drawing, f)?;
            schedules.push(json!({"drawing":d.drawing,"key":f.key,"kind":"fixture","room":f.room,"product":f.product,"source_entity_ids":f.entity_ids,"bounds_mm":bounds,"drawing_width_mm":bounds[1][0]-bounds[0][0],"drawing_depth_mm":bounds[1][1]-bounds[0][1],"footprint":"conservative union bounding rectangle of bound entities"}));
        }
    }
    let cell_width = elevations
        .iter()
        .map(|(_, w, _, _)| *w)
        .fold(1000.0, f64::max)
        + 600.0;
    let cell_height = elevations
        .iter()
        .map(|(_, _, h, _)| *h)
        .fold(2000.0, f64::max)
        + 800.0;
    let height = (elevations.len().div_ceil(3).max(1) as f64) * cell_height + 400.0;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {height}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><text x=\"50\" y=\"180\" font-size=\"120\">建具寸法模式図（製品の詳細姿図ではありません）</text>",
        cell_width * 3.0
    );
    for (i, (key, w, h, model)) in elevations.iter().enumerate() {
        let x = (i % 3) as f64 * cell_width + 150.0;
        let y = (i / 3) as f64 * cell_height + 450.0;
        svg.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"#345b77\" stroke-width=\"8\"/><text x=\"{x}\" y=\"{}\" font-size=\"90\">{} W{w} H{h}</text><text x=\"{x}\" y=\"{}\" font-size=\"70\">{}</text>",y+h+130.0,escape(key),y+h+230.0,escape(model)));
    }
    svg.push_str("</svg>");
    artifacts.push(("elevations.svg".into(), svg.into_bytes()));
    html.push_str("<p>修正候補は個別に作成したものです。同時適用せず、適用後に台帳を更新して再検査してください。CAD本体に建築の自動補完は追加していません。</p></html>");
    if before != cad_model::source_manifest(&project_root)?
        || ledger_bytes != fs::read(&args.ledger)?
    {
        return Err(
            "source or ledger changed during audit; retry without publishing stale results".into(),
        );
    }
    fs::create_dir(&out)?;
    write_json(&out.join("report.json"), &analysis.report)?;
    write_json(&out.join("repairs.json"), &repair_results)?;
    write_json(&out.join("schedule.json"), &schedules)?;
    write_json(
        &out.join("manifest.json"),
        &json!({"schema_version":SCHEMA,"project":project_root,"source_files":before,"ledger_blake3":blake3::hash(&ledger_bytes).to_hex().to_string(),"read_only":true,"proposals_applied":false}),
    )?;
    fs::write(out.join("index.html"), html)?;
    for (name, bytes) in artifacts {
        fs::write(out.join(name), bytes)?;
    }
    println!(
        "Audit {:?}: {} checks; {}",
        analysis.report.status,
        analysis.report.checks.len(),
        out.join("index.html").display()
    );
    Ok(analysis.report.status)
}
fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(Status::Pass) => ExitCode::SUCCESS,
        Ok(Status::Fail) => ExitCode::from(1),
        Ok(Status::Unknown) => ExitCode::from(2),
        Err(error) => {
            eprintln!("cad-audit: {error}");
            ExitCode::from(3)
        }
    }
}
