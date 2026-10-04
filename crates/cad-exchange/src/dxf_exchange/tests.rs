use super::*;
use cad_model::{BlockDefinition, BlockDefinitionConfig, EntityRecord};
use serde_json::json;

fn project() -> (tempfile::TempDir, ProjectSource) {
    let temp = tempfile::tempdir().unwrap();
    let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
        parent_dir: temp.path().display().to_string(),
        folder_name: "source".into(),
        project_name: "DXF tests".into(),
        drawing: "plan".into(),
        paper: "A3".into(),
        orientation: cad_model::SheetOrientation::Landscape,
        scale_denominator: 100,
    })
    .unwrap();
    let source = cad_model::load_project(created.project_path).unwrap();
    (temp, source)
}
fn entity(index: usize, geometry: serde_json::Value) -> EntityRecord {
    let mut value = geometry;
    value["schema_version"] = json!(cad_model::CURRENT_SCHEMA_VERSION);
    value["id"] = json!(format!("ent_{index:026}"));
    value["layer"] = json!("0-1");
    EntityRecord {
        line: index + 1,
        entity: serde_json::from_value(value).unwrap(),
    }
}
fn encoded(drawing: &Drawing) -> Vec<u8> {
    let mut bytes = Vec::new();
    drawing.save(&mut bytes).unwrap();
    bytes
}
#[test]
fn upright_annotations_expand_to_reported_positioned_text_and_strict_blocks() {
    let (_temp, mut source) = project();
    let style = source.styles.text_styles.keys().next().unwrap().clone();
    let metrics = source.styles.text_styles.get_mut(&style).unwrap();
    metrics.height = 10.;
    metrics.width = 5.;
    metrics.spacing = 2.;
    metrics.align = cad_model::TextAlign::Left;
    source.drawings[0].entities = vec![entity(
        1,
        json!({"type":"text","style":style,"at":[100,200],"rotation_deg":0,"value":"室名\nA2","writing_mode":"vertical_upright"}),
    )];
    let result = export(&source, "plan", false).unwrap();
    assert!(
        result
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "upright_text_expanded")
    );
    let bytes = result.bytes.unwrap();
    let drawing = Drawing::load(&mut std::io::Cursor::new(bytes)).unwrap();
    let text = drawing
        .entities()
        .map(|entity| match &entity.specific {
            dx::EntityType::Text(text) => text,
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(text.len(), 4);
    assert_eq!(
        text.iter()
            .map(|text| (text.location.x, text.location.y, text.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (100., 190., "室"),
            (100., 178., "名"),
            (93., 190., "A"),
            (93., 178., "2")
        ]
    );
    assert!(export(&source, "plan", true).unwrap().bytes.is_none());
}
#[test]
fn roundtrip_unicode_geometry_and_mirrored_block_without_mutating_source() {
    let (_temp, mut source) = project();
    let text_style = source.styles.text_styles.keys().next().unwrap().clone();
    source.blocks.insert(
        "part".into(),
        BlockDefinition {
            id: "part".into(),
            config: BlockDefinitionConfig {
                schema_version: cad_model::CURRENT_SCHEMA_VERSION.into(),
                name: "建具".into(),
                base_point: [10.0, 20.0],
            },
            entities: vec![entity(7, json!({"type":"line","p1":[10,20],"p2":[30,20]}))],
        },
    );
    source.drawings[0].entities = vec![
        entity(1, json!({"type":"line","p1":[10,20],"p2":[100,200]})),
        entity(2, json!({"type":"circle","center":[20,30],"radius":15})),
        entity(
            3,
            json!({"type":"text","at":[50,60],"style":text_style,"value":"壁・建具 🚪","rotation_deg":30}),
        ),
        entity(
            4,
            json!({"type":"block_ref","block":"part","at":[500,600],"scale":2,"rotation_deg":90,"mirror_x":true}),
        ),
        entity(
            5,
            json!({"type":"ellipse","center":[20,30],"radius_x":10,"radius_y":20,"rotation_deg":15,"start_deg":0,"end_deg":360}),
        ),
        entity(
            6,
            json!({"type":"polyline","points":[[0,0],[10,0],[10,10],[0,0]],"closed":true}),
        ),
    ];
    let before = cad_model::source_manifest(&source.root).unwrap();
    let output = export(&source, "plan", false).unwrap();
    assert!(output.report.blockers.is_empty(), "{:?}", output.report);
    let imported = import(&output.bytes.unwrap(), "roundtrip", None).unwrap();
    assert!(imported.report.blockers.is_empty(), "{:?}", imported.report);
    let restored = imported.source.as_ref().unwrap();
    assert!(cad_check::check_project(&restored.root).is_ok());
    assert_eq!(restored.drawings[0].entities.len(), 6);
    assert!(
        restored.drawings[0]
            .entities
            .iter()
            .any(|r| matches!(&r.entity,Entity::Text{value,..} if value == "壁・建具 🚪"))
    );
    assert!(restored.drawings[0].entities.iter().any(|r| matches!(&r.entity,Entity::BlockRef{at,scale,rotation_deg,mirror_x,..} if *at==[500.0,600.0] && *scale==2.0 && *rotation_deg==90.0 && *mirror_x)));
    let block = restored.blocks.values().next().unwrap();
    assert_eq!(block.config.base_point, [10.0, 20.0]);
    assert_eq!(cad_model::source_manifest(&source.root).unwrap(), before);
    assert!(export(&source, "plan", true).unwrap().bytes.is_none());
}
#[test]
fn units_and_legacy_polyline_preserve_vertices_and_closed_topology() {
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2013;
    drawing.header.default_drawing_units = Units::Meters;
    let mut poly = dx::Polyline::default();
    poly.set_is_closed(true);
    for p in [[0.0, 0.0], [2.0, 0.0], [2.0, 3.0]] {
        poly.add_vertex(
            &mut drawing,
            dx::Vertex {
                location: point(p),
                ..Default::default()
            },
        );
    }
    drawing.add_entity(dx::Entity::new(dx::EntityType::Polyline(poly)));
    let result = import(&encoded(&drawing), "meters", None).unwrap();
    assert!(result.report.blockers.is_empty(), "{:?}", result.report);
    let Entity::Polyline { points, closed, .. } =
        &result.source.as_ref().unwrap().drawings[0].entities[0].entity
    else {
        panic!("expected polyline")
    };
    assert!(*closed);
    assert_eq!(
        points,
        &vec![[0.0, 0.0], [2000.0, 0.0], [2000.0, 3000.0], [0.0, 0.0]]
    );
    drawing.header.default_drawing_units = Units::Unitless;
    assert!(
        import(&encoded(&drawing), "unitless", None)
            .unwrap()
            .source
            .is_none()
    );
    assert!(
        import(&encoded(&drawing), "override", Some(25.4))
            .unwrap()
            .source
            .is_some()
    );
}
#[test]
fn unknown_nonplanar_and_malformed_inputs_keep_blocked_reports() {
    let unknown = b"0\nSECTION\n2\nENTITIES\n0\nUNKNOWN_FUTURE\n10\n1\n0\nENDSEC\n0\nEOF\n";
    let result = import(unknown, "unknown", Some(1.0)).unwrap();
    assert!(result.source.is_none());
    assert_eq!(result.report.record_counts["UNKNOWN_FUTURE"], 1);
    assert_eq!(result.report.blockers[0].code, "unsupported_record");
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2013;
    drawing.header.default_drawing_units = Units::Millimeters;
    drawing.add_entity(dx::Entity::new(dx::EntityType::Line(dx::Line::new(
        DxfPoint::new(0.0, 0.0, 1.0),
        DxfPoint::new(1.0, 1.0, 1.0),
    ))));
    let result = import(&encoded(&drawing), "3D", None).unwrap();
    assert!(result.source.is_none());
    assert!(
        result
            .report
            .blockers
            .iter()
            .any(|b| b.code == "conversion_failed")
    );
    let malformed = import(b"broken", "broken", None).unwrap();
    assert_eq!(malformed.report.status, "blocked");
    assert!(malformed.report.source_blake3.is_some());
    let mut nonfinite = dx::Line::new(point([0.0, 0.0]), point([1.0, 1.0]));
    nonfinite.extrusion_direction = Vector::new(f64::NAN, 0.0, 1.0);
    let mut invalid_drawing = Drawing::new();
    invalid_drawing.header.version = AcadVersion::R2013;
    invalid_drawing.header.default_drawing_units = Units::Millimeters;
    invalid_drawing.add_entity(dx::Entity::new(dx::EntityType::Line(nonfinite)));
    assert!(
        import(&encoded(&invalid_drawing), "normal", None)
            .unwrap()
            .source
            .is_none()
    );
}
#[test]
fn solid_holes_export_as_triangles_with_matching_area() {
    let (_temp, mut source) = project();
    source.drawings[0].entities = vec![entity(
        1,
        json!({"type":"hatch","loops":[[[0,0],[10,0],[10,10],[0,10],[0,0]],[[2,2],[2,8],[8,8],[8,2],[2,2]]],"pattern":"solid","fill":"black","angle_deg":0,"scale":10}),
    )];
    let result = export(&source, "plan", false).unwrap();
    let drawing = Drawing::load(&mut std::io::Cursor::new(result.bytes.unwrap())).unwrap();
    let mut area = 0.0;
    let mut triangles = 0;
    let mut boundaries = 0;
    for entity in drawing.entities() {
        let dx::EntityType::Solid(s) = &entity.specific else {
            assert!(matches!(entity.specific, dx::EntityType::Line(_)));
            boundaries += 1;
            continue;
        };
        triangles += 1;
        area += ((s.second_corner.x - s.first_corner.x) * (s.third_corner.y - s.first_corner.y)
            - (s.second_corner.y - s.first_corner.y) * (s.third_corner.x - s.first_corner.x))
            .abs()
            / 2.0;
    }
    assert!(triangles > 0);
    assert_eq!(boundaries, 8);
    assert!((area - 64.0).abs() < 1e-8);
    assert!(!result.report.warnings.is_empty());
}

#[test]
fn per_entity_and_global_linetype_scaling_is_preserved_at_import_layout_scale() {
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2013;
    drawing.header.default_drawing_units = Units::Millimeters;
    drawing.header.line_type_scale = 2.0;
    drawing.add_line_type(tables::LineType {
        name: "DASHED".into(),
        element_count: 2,
        total_pattern_length: 30.0,
        dash_dot_space_lengths: vec![20.0, -10.0],
        ..Default::default()
    });
    let mut line = dx::Entity::new(dx::EntityType::Line(dx::Line::new(
        point([0.0, 0.0]),
        point([1000.0, 0.0]),
    )));
    line.common.line_type_name = "DASHED".into();
    line.common.line_type_scale = 3.0;
    drawing.add_entity(line);
    let imported = import(&encoded(&drawing), "dash", None).unwrap();
    assert!(imported.report.blockers.is_empty(), "{:?}", imported.report);
    let source = imported.source.as_ref().unwrap();
    let stroke =
        cad_model::resolve_entity_stroke(source, &source.drawings[0].entities[0].entity).unwrap();
    assert_eq!(stroke.dash.len(), 2);
    assert!((stroke.dash[0] - 1.2).abs() < 1e-10);
    assert!((stroke.dash[1] - 0.6).abs() < 1e-10);
}
