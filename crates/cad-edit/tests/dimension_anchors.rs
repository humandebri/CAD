use cad_edit::{DrawingEditRequest, DrawingEditResult, EditOperation, ProjectTemplateRequest};
use cad_model::SheetOrientation;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    layer: String,
    style: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        cad_edit::create_project(&ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "project".into(),
            project_name: "Dimension anchors".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let project = cad_model::load_project(&root).unwrap();
        Self {
            _temp: temp,
            root,
            layer: project.layers.active_layer.unwrap(),
            style: project
                .styles
                .dimension_styles
                .keys()
                .next()
                .unwrap()
                .clone(),
        }
    }

    fn request(&self, operation: Value) -> DrawingEditRequest {
        let project = cad_model::load_project(&self.root).unwrap();
        DrawingEditRequest {
            drawing: "plan".into(),
            expected_revision: cad_edit::editor_state(&project, "plan").unwrap().revision,
            operation: serde_json::from_value::<EditOperation>(operation).unwrap(),
        }
    }

    fn apply(&self, operation: Value) -> DrawingEditResult {
        cad_edit::apply_edit(&self.root, &self.request(operation)).unwrap()
    }

    fn create(&self, mut entity: Value) -> String {
        entity["layer"] = json!(self.layer);
        self.apply(json!({"kind":"create","entity":entity}))
            .entity_ids[0]
            .clone()
    }

    fn dimension(&self, source: &str, feature: &str, index: usize) -> String {
        let project = cad_model::load_project(&self.root).unwrap();
        let anchor = json!({"kind":"entity","entity_id":source,"feature":feature,"index":index});
        let point = cad_model::resolve_dimension_anchor(
            &serde_json::from_value(anchor.clone()).unwrap(),
            &project.drawings[0].entities,
        )
        .unwrap();
        self.create(json!({
            "type":"dimension","style":self.style,"p1":point,"p2":[500,300],
            "offset":30,"value":null,
            "measurement":{"kind":"horizontal","first":anchor,"second":{"kind":"fixed","point":[500,300]}}
        }))
    }

    fn entity(&self, id: &str) -> Value {
        let project = cad_model::load_project(&self.root).unwrap();
        project.drawings[0]
            .entities
            .iter()
            .find(|r| r.entity.id().as_str() == id)
            .map(|r| serde_json::to_value(&r.entity).unwrap())
            .unwrap_or(Value::Null)
    }

    fn anchor_point(&self, dimension: &str) -> [f64; 2] {
        let project = cad_model::load_project(&self.root).unwrap();
        let anchor = self.entity(dimension)["measurement"]["first"].clone();
        cad_model::resolve_dimension_anchor(
            &serde_json::from_value(anchor).unwrap(),
            &project.drawings[0].entities,
        )
        .unwrap()
    }

    fn assert_requires_resolution(&self, operation: &Value, dimension: &str) {
        let request = self.request(operation.clone());
        let path = self.root.join("drawings/plan/entities.ndjson");
        let before = std::fs::read(&path).unwrap();
        let preview = cad_edit::preview_edit(&self.root, &request).unwrap();
        assert_eq!(preview.dimension_impacts, [dimension]);
        let error = cad_edit::apply_edit(&self.root, &request).unwrap_err();
        assert!(
            error.to_string().contains("dimension_resolution_required"),
            "{error}"
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

fn polyline() -> Value {
    json!({"type":"polyline","points":[[0,0],[100,0],[100,100],[200,100],[200,200]],"closed":false})
}

fn trim(source: &str, cutter: &str, pick: [f64; 2]) -> Value {
    json!({"kind":"trim","target_entity_id":source,"cutter_entity_id":cutter,"pick_point":pick})
}

#[test]
fn trim_remaps_retained_and_shortened_polyline_midpoints() {
    let f = Fixture::new();
    let source = f.create(polyline());
    let cutter = f.create(json!({"type":"line","p1":[50,50],"p2":[150,50]}));
    let retained = f.dimension(&source, "midpoint", 2);
    let shortened = f.dimension(&source, "midpoint", 1);
    let operation = trim(&source, &cutter, [50.0, 0.0]);
    let preview = cad_edit::preview_edit(&f.root, &f.request(operation.clone())).unwrap();
    assert!(preview.dimension_impacts.is_empty());
    f.apply(operation);
    for (id, index, point) in [
        (&retained, 1, [150.0, 100.0]),
        (&shortened, 0, [100.0, 75.0]),
    ] {
        assert_eq!(f.entity(id)["measurement"]["first"]["index"], index);
        assert_eq!(f.anchor_point(id), point);
        assert_eq!(
            *preview.entities.iter().find(|e| e["id"] == *id).unwrap(),
            f.entity(id)
        );
    }
    // Ordinary coordinate edits must keep following the same segment.
    f.apply(json!({"kind":"translate","entity_id":source,"delta":[10,20],"duplicate":false}));
    assert_eq!(f.anchor_point(&retained), [160.0, 120.0]);
    assert!(cad_check::check_project(&f.root).is_ok());
}

#[test]
fn trim_remaps_midpoint_to_the_retained_piece_with_a_new_id() {
    let f = Fixture::new();
    let source = f.create(polyline());
    let cutter = f.create(json!({"type":"circle","center":[150,100],"radius":20}));
    let dimension = f.dimension(&source, "midpoint", 3);
    f.apply(trim(&source, &cutter, [150.0, 100.0]));
    let anchor = &f.entity(&dimension)["measurement"]["first"];
    assert_ne!(anchor["entity_id"], source);
    assert_eq!(anchor["index"], 1);
    assert_eq!(f.anchor_point(&dimension), [200.0, 150.0]);
    assert!(cad_check::check_project(&f.root).is_ok());
}

#[test]
fn trim_remaps_the_closing_segment_even_when_vertex_count_is_unchanged() {
    let f = Fixture::new();
    let source = f.create(
        json!({"type":"polyline","points":[[0,0],[100,0],[100,100],[0,100],[0,0]],"closed":true}),
    );
    let cutter = f.create(json!({"type":"line","p1":[40,110],"p2":[110,40]}));
    let dimension = f.dimension(&source, "midpoint", 3);
    f.apply(trim(&source, &cutter, [100.0, 100.0]));
    assert_eq!(f.entity(&source)["points"].as_array().unwrap().len(), 5);
    assert_eq!(f.entity(&source)["closed"], false);
    assert_eq!(f.entity(&dimension)["measurement"]["first"]["index"], 1);
    assert_eq!(f.anchor_point(&dimension), [0.0, 50.0]);
    assert!(cad_check::check_project(&f.root).is_ok());
}

#[test]
fn splitting_a_two_vertex_polyline_requires_midpoint_resolution() {
    let f = Fixture::new();
    let source = f.create(json!({"type":"polyline","points":[[0,0],[100,0]],"closed":false}));
    let cutter = f.create(json!({"type":"circle","center":[50,0],"radius":20}));
    let dimension = f.dimension(&source, "midpoint", 0);
    // Both pieces have the same vertex count as the original path.
    f.assert_requires_resolution(&trim(&source, &cutter, [50.0, 0.0]), &dimension);
}

#[test]
fn trim_midpoints_with_deleted_or_ambiguous_segments_require_resolution() {
    for split in [false, true] {
        for action in ["detach", "delete"] {
            let f = Fixture::new();
            let source = f.create(polyline());
            let cutter = f.create(if split {
                json!({"type":"circle","center":[150,100],"radius":20})
            } else {
                json!({"type":"line","p1":[50,50],"p2":[150,50]})
            });
            let dimension = f.dimension(&source, "midpoint", if split { 2 } else { 0 });
            let before = f.anchor_point(&dimension);
            let operation = trim(
                &source,
                &cutter,
                if split { [150.0, 100.0] } else { [50.0, 0.0] },
            );
            f.assert_requires_resolution(&operation, &dimension);
            f.apply(json!({"kind":"resolve_dimensions","operation":operation,"resolutions":[{"entity_id":dimension,"action":action}]}));
            if action == "delete" {
                assert_eq!(f.entity(&dimension), Value::Null);
            } else {
                assert_eq!(
                    f.entity(&dimension)["measurement"]["first"]["kind"],
                    "fixed"
                );
                assert_eq!(f.anchor_point(&dimension), before);
            }
            assert!(cad_check::check_project(&f.root).is_ok());
        }
    }
}

#[test]
fn removing_curve_quadrants_requires_resolution_before_saving() {
    for ellipse in [false, true] {
        for action in ["detach", "delete"] {
            let f = Fixture::new();
            let source = f.create(if ellipse {
                json!({"type":"ellipse","center":[1000,1000],"radius_x":100,"radius_y":50,"rotation_deg":30,"start_deg":0,"end_deg":270})
            } else {
                json!({"type":"arc","center":[1000,1000],"radius":100,"start_deg":0,"end_deg":270})
            });
            let cutter = f.create(json!({"type":"line","p1":[1000,850],"p2":[1000,1150]}));
            let dimension = f.dimension(&source, "quadrant", 2);
            let before = f.anchor_point(&dimension);
            let operation = if ellipse {
                let mut entity = f.entity(&source);
                entity["end_deg"] = json!(90);
                json!({"kind":"replace","entity_id":source,"entity":entity})
            } else {
                trim(&source, &cutter, [900.0, 1000.0])
            };
            f.assert_requires_resolution(&operation, &dimension);
            f.apply(json!({"kind":"resolve_dimensions","operation":operation,"resolutions":[{"entity_id":dimension,"action":action}]}));
            if action == "delete" {
                assert_eq!(f.entity(&dimension), Value::Null);
            } else {
                assert_eq!(
                    f.entity(&dimension)["measurement"]["first"]["kind"],
                    "fixed"
                );
                assert!(
                    f.anchor_point(&dimension)
                        .into_iter()
                        .zip(before)
                        .all(|(a, b)| (a - b).abs() < 1e-9)
                );
            }
            assert!(cad_check::check_project(&f.root).is_ok());
        }
    }
}
