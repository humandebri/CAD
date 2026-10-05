use cad_model::{Entity, EntityRecord};
use cad_plan_audit::{Ledger, Status, analyze, repair_candidate};
use serde_json::{Value, json};
use std::{fs, process::Command};

fn fixture() -> (tempfile::TempDir, cad_model::ProjectSource, Ledger) {
    let temp = tempfile::tempdir().unwrap();
    let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
        parent_dir: temp.path().display().to_string(),
        folder_name: "cad".into(),
        project_name: "Audit tests".into(),
        drawing: "plan".into(),
        paper: "A3".into(),
        orientation: cad_model::SheetOrientation::Landscape,
        scale_denominator: 50,
    })
    .unwrap();
    let mut source = cad_model::load_project(&created.project_path).unwrap();
    let fill = source.styles.colors.keys().next().unwrap().clone();
    let mut es = vec![];
    for (index, (x1, x2)) in [(0., 400.), (900., 2000.)].into_iter().enumerate() {
        let points = json!([[x1, 0], [x2, 0], [x2, 150], [x1, 150]]);
        es.push(make(index,json!({"type":"hatch","pattern":"solid","angle_deg":0,"scale":1,"fill":fill,"loops":[points]})));
        es.push(make(index+2,json!({"type":"polyline","closed":true,"points":[[x1,0],[x2,0],[x2,150],[x1,150],[x1,0]]})));
    }
    es.push(make(
        4,
        json!({"type":"arc","center":[400,150],"radius":500,"start_deg":0,"end_deg":90}),
    ));
    for (i, x) in [(5, 1600), (6, 1800)] {
        es.push(make(i,json!({"type":"polyline","closed":true,"points":[[x,300],[x+100,300],[x+100,400],[x,400],[x,300]]})));
    }
    source.drawings[0].entities = es;
    fs::write(
        std::path::Path::new(&created.project_path).join("drawings/plan/entities.ndjson"),
        source.drawings[0]
            .entities
            .iter()
            .map(|e| serde_json::to_string(&e.entity).unwrap() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    assert_eq!(
        cad_check::check_project(&created.project_path).status,
        cad_check::CheckStatus::Ok
    );
    let ledger:Ledger=serde_json::from_value(json!({"schema_version":"cad-plan-audit/1","tolerance_mm":0.1,"drawings":[{
        "drawing":"plan","walls":{"fill_entity_ids":[id(0),id(1)],"outline_entity_ids":[id(2),id(3)]},
        "rooms":[{"key":"room","inner_polygon":[[0,150],[2000,150],[2000,1500],[0,1500]]}],
        "fixtures":[{"key":"F1","entity_ids":[id(5)],"room":"room"},{"key":"F2","entity_ids":[id(6)],"room":"room"}],
        "openings":[{"key":"O1","axis":[[400,75],[900,75]],"depth_mm":150}],
        "doors":[{"key":"D1","opening":"O1","swing_entity_id":id(4),"closed_end":"start","product":{"name":"test","model":"test","source":"test fixture","nominal_height_mm":2000}}],
        "clearances":[{"kind":"between","key":"aisle","first":"F1","second":"F2","axis":"x","min_mm":80,"basis":"test project requirement"}],
        "installations":[{"key":"installation","fixture":"F1","room":"room","source":"test manufacturer requirement","required_width_mm":1000,"required_depth_mm":1000,"required_height_mm":2500,"available_height_mm":2600}]
    }]})).unwrap();
    (temp, source, ledger)
}
fn id(i: usize) -> String {
    format!("ent_{i:026}")
}
fn make(i: usize, mut v: Value) -> EntityRecord {
    v["schema_version"] = json!("0.3");
    v["id"] = json!(id(i));
    v["layer"] = json!("0-1");
    EntityRecord {
        line: i + 1,
        entity: serde_json::from_value(v).unwrap(),
    }
}
fn move_door(source: &mut cad_model::ProjectSource, delta: f64) {
    for r in &mut source.drawings[0].entities {
        if r.entity.id().as_str() == id(4)
            && let Entity::Arc { center, .. } = &mut r.entity
        {
            center[0] += delta;
        }
    }
}

#[test]
fn valid_ledger_is_deterministic_and_does_not_mutate_source() {
    let (_temp, source, ledger) = fixture();
    let snapshot = source.clone();
    let a = analyze(&source, &ledger).unwrap();
    let b = analyze(&source, &ledger).unwrap();
    assert_eq!(a.report.status, Status::Pass);
    assert_eq!(
        serde_json::to_value(a.report).unwrap(),
        serde_json::to_value(b.report).unwrap()
    );
    assert_eq!(source, snapshot);
}
#[test]
fn moved_door_produces_checked_deterministic_repair_without_applying() {
    let (temp, mut source, ledger) = fixture();
    move_door(&mut source, 600.0);
    let a = analyze(&source, &ledger).unwrap();
    assert_eq!(a.report.status, Status::Fail);
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "door.opening_alignment" && c.status == Status::Fail)
    );
    let repair = &a.repairs[0];
    assert!(repair.available);
    assert_eq!(repair.new_axis, [[1000.0, 75.0], [1500.0, 75.0]]);
    let files = cad_model::source_manifest(&source.root).unwrap();
    let revision =
        blake3::hash(&fs::read(source.root.join("drawings/plan/entities.ndjson")).unwrap())
            .to_hex()
            .to_string();
    let candidate = repair_candidate(&source, &ledger, repair, &revision, files.clone()).unwrap();
    assert_eq!(
        analyze(&candidate.project, &candidate.ledger)
            .unwrap()
            .report
            .status,
        Status::Pass
    );
    let again = repair_candidate(&source, &ledger, repair, &revision, files.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(candidate.request).unwrap(),
        serde_json::to_value(again.request).unwrap()
    );
    assert_eq!(cad_model::source_manifest(&source.root).unwrap(), files);
    assert!(temp.path().join("cad").exists());
}
#[test]
fn insufficient_installation_height_is_unknown_not_ceiling_height() {
    let (_temp, source, mut ledger) = fixture();
    ledger.drawings[0].installations[0].available_height_mm = None;
    let a = analyze(&source, &ledger).unwrap();
    assert_eq!(a.report.status, Status::Unknown);
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "installation.height" && c.status == Status::Unknown)
    );
}
#[test]
fn room_clearance_limits_follow_geometry_and_basis() {
    let (_temp, mut source, mut ledger) = fixture();
    if let Entity::Polyline { points, .. } = &mut source.drawings[0]
        .entities
        .iter_mut()
        .find(|r| r.entity.id().as_str() == id(6))
        .unwrap()
        .entity
    {
        for p in points {
            p[0] -= 50.;
        }
    }
    let a = analyze(&source, &ledger).unwrap();
    assert!(a.report.checks.iter().any(|c| c.code == "clearance.minimum"
        && c.status == Status::Fail
        && c.measured == Some(50.)));
    if let cad_plan_audit::Clearance::Between { basis, .. } = &mut ledger.drawings[0].clearances[0]
    {
        *basis = String::new();
    }
    assert!(
        analyze(&source, &ledger)
            .unwrap()
            .report
            .checks
            .iter()
            .any(|c| c.code == "clearance.minimum" && c.status == Status::Unknown)
    );
}
#[test]
fn missing_ambiguous_or_invalid_bindings_block() {
    let (_temp, source, mut ledger) = fixture();
    ledger.drawings[0].doors[0].swing_entity_id = "missing".into();
    assert!(analyze(&source, &ledger).is_err());
    ledger.drawings[0].doors[0].swing_entity_id = id(5);
    assert!(analyze(&source, &ledger).is_err());
    ledger.drawings[0].doors[0].swing_entity_id = id(4);
    ledger.drawings[0].openings[0].axis = [[0., 0.], [0., 0.]];
    assert!(analyze(&source, &ledger).is_err());
}
#[test]
fn repair_refuses_outside_wall_and_other_opening() {
    let (_temp, mut source, mut ledger) = fixture();
    move_door(&mut source, 1800.);
    assert!(!analyze(&source, &ledger).unwrap().repairs[0].available);
    move_door(&mut source, -1200.);
    ledger.drawings[0].openings.push(cad_plan_audit::Opening {
        key: "O2".into(),
        axis: [[1200., 75.], [1400., 75.]],
        depth_mm: 150.,
    });
    assert!(!analyze(&source, &ledger).unwrap().repairs[0].available);
}
#[test]
fn wall_repair_preserves_and_reports_existing_fixture_collision() {
    let (_temp, mut source, ledger) = fixture();
    move_door(&mut source, 900.);
    let a = analyze(&source, &ledger).unwrap();
    let repair = &a.repairs[0];
    assert!(repair.available);
    // The moved sector already collides with F1; closing/cutting the wall must not claim to fix it.
    let revision = "test-revision";
    let result = repair_candidate(&source, &ledger, repair, revision, vec![]);
    // A wall-only candidate can retain a pre-existing furniture failure, explicitly in after-report.
    let c = result.unwrap();
    assert_eq!(
        analyze(&c.project, &c.ledger).unwrap().report.status,
        Status::Fail
    );
}
#[test]
fn cli_preserves_canonical_data_and_refuses_overwrite_and_source_output() {
    let (temp, source, ledger) = fixture();
    let path = temp.path().join("ledger.json");
    fs::write(&path, serde_json::to_vec(&ledger).unwrap()).unwrap();
    let out = temp.path().join("audit");
    let before = cad_model::source_manifest(&source.root).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_cad-audit"))
            .arg(&source.root)
            .arg("--ledger")
            .arg(&path)
            .arg("--out")
            .arg(&out)
            .arg("--proposals")
            .output()
            .unwrap()
    };
    let r = run();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(out.join("report.json").exists());
    assert!(out.join("schedule.json").exists());
    assert!(out.join("elevations.svg").exists());
    assert_eq!(before, cad_model::source_manifest(&source.root).unwrap());
    assert_eq!(run().status.code(), Some(3));
    let bad = Command::new(env!("CARGO_BIN_EXE_cad-audit"))
        .arg(&source.root)
        .arg("--ledger")
        .arg(&path)
        .arg("--out")
        .arg(source.root.join("audit"))
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(3));
    assert!(!source.root.join("audit").exists());
}
#[test]
fn source_checked_request_rejects_stale_project() {
    let (_temp, mut source, ledger) = fixture();
    move_door(&mut source, 600.);
    // Publish the moved door only into this isolated fixture.
    fs::write(
        source.root.join("drawings/plan/entities.ndjson"),
        source.drawings[0]
            .entities
            .iter()
            .map(|r| serde_json::to_string(&r.entity).unwrap() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let files = cad_model::source_manifest(&source.root).unwrap();
    let revision =
        blake3::hash(&fs::read(source.root.join("drawings/plan/entities.ndjson")).unwrap())
            .to_hex()
            .to_string();
    let repair = &analyze(&source, &ledger).unwrap().repairs[0];
    let candidate = repair_candidate(&source, &ledger, repair, &revision, files).unwrap();
    let preview = cad_edit::preview_edit(&source.root, &candidate.request).unwrap();
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
    fs::write(
        source.root.join("cad.project.toml"),
        "schema_version = \"0.3\"\nname = \"changed\"\n",
    )
    .unwrap();
    assert!(cad_edit::preview_edit(&source.root, &candidate.request).is_err());
}

#[test]
fn repair_rejects_new_fixture_collision_and_mixed_wall_styles() {
    let (_temp, mut source, mut ledger) = fixture();
    source.drawings[0].entities.push(make(7,json!({"type":"polyline","closed":true,"points":[[500,10],[600,10],[600,100],[500,100],[500,10]]})));
    ledger.drawings[0].fixtures.push(cad_plan_audit::Fixture {
        key: "F3".into(),
        entity_ids: vec![id(7)],
        room: None,
        wall_mounted: false,
        product: None,
    });
    move_door(&mut source, 600.);
    let a = analyze(&source, &ledger).unwrap();
    assert!(a.repairs[0].available);
    let error = match repair_candidate(&source, &ledger, &a.repairs[0], "test", vec![]) {
        Ok(_) => panic!("closing the old opening must collide with F3"),
        Err(e) => e,
    };
    assert!(error.to_string().contains("new failed check"));
    ledger.drawings[0].fixtures.pop();
    if let Entity::Hatch { angle_deg, .. } = &mut source.drawings[0]
        .entities
        .iter_mut()
        .find(|e| e.entity.id().as_str() == id(1))
        .unwrap()
        .entity
    {
        *angle_deg = 45.;
    }
    assert!(repair_candidate(&source, &ledger, &a.repairs[0], "test", vec![]).is_err());
}

#[test]
fn detached_old_opening_cannot_create_wall_or_pass_attachment() {
    let (_temp, mut source, mut ledger) = fixture();
    ledger.drawings[0].openings[0].axis = [[2500., 75.], [3000., 75.]];
    move_door(&mut source, 600.);
    let a = analyze(&source, &ledger).unwrap();
    assert!(!a.repairs[0].available);
    assert!(repair_candidate(&source, &ledger, &a.repairs[0], "test", vec![]).is_err());
    move_door(&mut source, 1500.);
    let a = analyze(&source, &ledger).unwrap();
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "opening.wall_attachment" && c.status == Status::Fail)
    );
}

fn add_wall(source: &mut cad_model::ProjectSource, ledger: &mut Ledger, i: usize, rect: [f64; 4]) {
    let [x1, y1, x2, y2] = rect;
    let fill = source.styles.colors.keys().next().unwrap().clone();
    source.drawings[0].entities.push(make(i,json!({"type":"hatch","pattern":"solid","angle_deg":0,"scale":1,"fill":fill,"loops":[[[x1,y1],[x2,y1],[x2,y2],[x1,y2]]]})));
    source.drawings[0].entities.push(make(
        i + 1,
        json!({"type":"polyline","closed":true,"points":[[x1,y1],[x2,y1],[x2,y2],[x1,y2],[x1,y1]]}),
    ));
    ledger.drawings[0].walls.fill_entity_ids.push(id(i));
    ledger.drawings[0].walls.outline_entity_ids.push(id(i + 1));
}

#[test]
fn partitioned_room_does_not_pass_installation_width() {
    let (_temp, mut source, mut ledger) = fixture();
    add_wall(&mut source, &mut ledger, 8, [900., 150., 1000., 1500.]);
    ledger.drawings[0].installations[0].required_width_mm = Some(1200.);
    let a = analyze(&source, &ledger).unwrap();
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "room.wall_overlap" && c.status == Status::Fail)
    );
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "installation.width" && c.status == Status::Unknown)
    );
}

#[test]
fn registered_opposing_doors_detect_swing_collision() {
    let (_temp, mut source, mut ledger) = fixture();
    add_wall(&mut source, &mut ledger, 8, [0., 650., 400., 800.]);
    add_wall(&mut source, &mut ledger, 10, [900., 650., 2000., 800.]);
    source.drawings[0].entities.push(make(
        12,
        json!({"type":"arc","center":[400,650],"radius":500,"start_deg":0,"end_deg":-90}),
    ));
    ledger.drawings[0].openings.push(cad_plan_audit::Opening {
        key: "O2".into(),
        axis: [[400., 725.], [900., 725.]],
        depth_mm: 150.,
    });
    ledger.drawings[0].doors.push(cad_plan_audit::Door {
        key: "D2".into(),
        opening: "O2".into(),
        swing_entity_id: id(12),
        closed_end: cad_plan_audit::ClosedEnd::Start,
        product: None,
    });
    let a = analyze(&source, &ledger).unwrap();
    assert!(
        a.report
            .checks
            .iter()
            .any(|c| c.code == "door.swing_door_overlap"
                && c.status == Status::Fail
                && c.measured.unwrap() > 1.)
    );
}

#[test]
fn installation_rejects_conflicting_room_and_checks_unbound_fixture_containment() {
    let (_temp, source, mut ledger) = fixture();
    ledger.drawings[0].rooms.push(cad_plan_audit::Room {
        key: "unrelated".into(),
        inner_polygon: vec![
            [3000., 3000.],
            [5000., 3000.],
            [5000., 5000.],
            [3000., 5000.],
        ],
    });
    ledger.drawings[0].installations[0].room = "unrelated".into();
    assert!(
        analyze(&source, &ledger)
            .unwrap_err()
            .to_string()
            .contains("conflicts")
    );
    ledger.drawings[0].fixtures[0].room = None;
    assert!(
        analyze(&source, &ledger)
            .unwrap_err()
            .to_string()
            .contains("outside")
    );
    ledger.drawings[0].installations[0].room = "room".into();
    assert_eq!(
        analyze(&source, &ledger).unwrap().report.status,
        Status::Pass
    );
    ledger.drawings[0].rooms[0].inner_polygon =
        vec![[1650., 150.], [2000., 150.], [2000., 1500.], [1650., 1500.]];
    assert!(
        analyze(&source, &ledger)
            .unwrap_err()
            .to_string()
            .contains("outside")
    );
}

#[test]
fn disjoint_wall_hatch_loops_support_checks_and_repair() {
    let (_temp, mut source, mut ledger) = fixture();
    let second = match &source.drawings[0]
        .entities
        .iter()
        .find(|r| r.entity.id().as_str() == id(1))
        .unwrap()
        .entity
    {
        Entity::Hatch { loops, .. } => loops[0].clone(),
        _ => unreachable!(),
    };
    if let Entity::Hatch { loops, .. } = &mut source.drawings[0]
        .entities
        .iter_mut()
        .find(|r| r.entity.id().as_str() == id(0))
        .unwrap()
        .entity
    {
        loops.push(second);
    }
    source.drawings[0]
        .entities
        .retain(|r| r.entity.id().as_str() != id(1));
    ledger.drawings[0].walls.fill_entity_ids = vec![id(0)];
    assert_eq!(
        cad_check::check_loaded_project(&source).status,
        cad_check::CheckStatus::Ok
    );
    assert_eq!(
        analyze(&source, &ledger).unwrap().report.status,
        Status::Pass
    );
    move_door(&mut source, 600.);
    let repair = analyze(&source, &ledger).unwrap().repairs.remove(0);
    let candidate = repair_candidate(&source, &ledger, &repair, "test", vec![]).unwrap();
    assert_eq!(
        analyze(&candidate.project, &candidate.ledger)
            .unwrap()
            .report
            .status,
        Status::Pass
    );
}

#[test]
fn wall_hatch_evenodd_fill_preserves_holes_islands_and_loop_order() {
    let (_temp, mut source, mut ledger) = fixture();
    let loops = vec![
        vec![[200., 200.], [800., 200.], [800., 800.], [200., 800.]],
        vec![[400., 400.], [600., 400.], [600., 600.], [400., 600.]],
        vec![[0., 0.], [1000., 0.], [1000., 1000.], [0., 1000.]],
        vec![[2000., 0.], [3000., 0.], [3000., 1000.], [2000., 1000.]],
    ];
    if let Entity::Hatch { loops: target, .. } = &mut source.drawings[0].entities[0].entity {
        *target = loops;
    }
    source.drawings[0]
        .entities
        .retain(|r| r.entity.id().as_str() == id(0));
    let d = &mut ledger.drawings[0];
    d.walls.fill_entity_ids = vec![id(0)];
    d.walls.outline_entity_ids.clear();
    d.rooms.clear();
    d.fixtures.clear();
    d.openings.clear();
    d.doors.clear();
    d.clearances.clear();
    d.installations.clear();
    for (i, x) in [(8, 250.), (9, 450.), (10, 2100.)] {
        source.drawings[0].entities.push(make(i, json!({"type":"polyline","closed":true,"points":[[x,450.],[x+50.,450.],[x+50.,500.],[x,500.],[x,450.]]})));
        d.fixtures.push(cad_plan_audit::Fixture {
            key: format!("F{i}"),
            entity_ids: vec![id(i)],
            room: None,
            wall_mounted: false,
            product: None,
        });
    }
    assert_eq!(
        cad_check::check_loaded_project(&source).status,
        cad_check::CheckStatus::Ok
    );
    for _ in 0..2 {
        let a = analyze(&source, &ledger).unwrap();
        for (key, expected) in [
            ("F8", Status::Pass),
            ("F9", Status::Fail),
            ("F10", Status::Fail),
        ] {
            let c = a
                .report
                .checks
                .iter()
                .find(|c| c.object == key && c.code == "fixture.wall_overlap")
                .unwrap();
            assert_eq!(c.status, expected);
            assert_eq!(
                c.measured,
                Some(if expected == Status::Pass { 0. } else { 2500. })
            );
        }
        if let Entity::Hatch { loops, .. } = &mut source.drawings[0].entities[0].entity {
            loops.reverse();
            for ring in loops {
                ring.reverse();
            }
        }
    }
}
