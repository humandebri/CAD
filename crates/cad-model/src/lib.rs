//! Core CAD source model crate.
//!
//! This crate owns the typed representation of the NDJSON/TOML source files.
//! Phase 1 reads the local project layout into strict Rust types without
//! checker, renderer, or diff behavior.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use ulid::Ulid;

pub const CRATE_NAME: &str = "cad-model";
pub const CURRENT_SCHEMA_VERSION: &str = "0.1";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to list {path}")]
    ListDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TOML {path}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to parse NDJSON {path}:{line}")]
    Ndjson {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("empty NDJSON line at {path}:{line}")]
    EmptyNdjsonLine { path: PathBuf, line: usize },
    #[error("unsupported schema_version {found:?} in {path}; expected {expected:?}")]
    UnsupportedSchema {
        path: PathBuf,
        line: Option<usize>,
        found: String,
        expected: &'static str,
    },
    #[error("invalid entity id {value:?}; expected ent_<26-character ULID>")]
    InvalidEntityId { value: String },
}

pub type ModelResult<T> = Result<T, ModelError>;
pub type Point = [f64; 2];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    raw: String,
    ulid: Ulid,
}

impl EntityId {
    pub fn parse(value: &str) -> ModelResult<Self> {
        let Some(encoded) = value.strip_prefix("ent_") else {
            return Err(ModelError::InvalidEntityId {
                value: value.to_owned(),
            });
        };
        let Ok(ulid) = Ulid::from_string(encoded) else {
            return Err(ModelError::InvalidEntityId {
                value: value.to_owned(),
            });
        };
        Ok(Self {
            raw: value.to_owned(),
            ulid,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    #[must_use]
    pub fn ulid(&self) -> Ulid {
        self.ulid
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl Serialize for EntityId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayerRules {
    pub layers: BTreeMap<String, LayerDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayerDef {
    pub name: String,
    pub visible: bool,
    pub printable: bool,
    pub color: String,
    pub line_type: String,
    pub line_width: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StyleRules {
    pub colors: BTreeMap<String, ColorDef>,
    pub line_types: BTreeMap<String, LineTypeDef>,
    pub text_styles: BTreeMap<String, TextStyleDef>,
    pub dimension_styles: BTreeMap<String, DimensionStyleDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ColorDef {
    pub rgb: String,
    pub print_width: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LineTypeDef {
    pub dash: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextStyleDef {
    pub font_family: String,
    pub height: f64,
    pub align: TextAlign,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DimensionStyleDef {
    pub text_style: String,
    pub arrow_size: f64,
    pub extension_gap: f64,
    pub precision: u8,
    pub unit: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SheetConfig {
    pub schema_version: String,
    pub paper: String,
    pub orientation: SheetOrientation,
    pub scale: String,
    pub origin: Point,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SheetOrientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Entity {
    Line {
        schema_version: String,
        id: EntityId,
        layer: String,
        p1: Point,
        p2: Point,
    },
    Polyline {
        schema_version: String,
        id: EntityId,
        layer: String,
        points: Vec<Point>,
        closed: bool,
    },
    Arc {
        schema_version: String,
        id: EntityId,
        layer: String,
        center: Point,
        radius: f64,
        start_deg: f64,
        end_deg: f64,
    },
    Circle {
        schema_version: String,
        id: EntityId,
        layer: String,
        center: Point,
        radius: f64,
    },
    Text {
        schema_version: String,
        id: EntityId,
        layer: String,
        style: String,
        at: Point,
        rotation_deg: f64,
        value: String,
    },
    Dimension {
        schema_version: String,
        id: EntityId,
        layer: String,
        style: String,
        p1: Point,
        p2: Point,
        offset: f64,
        value: Option<String>,
    },
    BlockRef {
        schema_version: String,
        id: EntityId,
        layer: String,
        block: String,
        at: Point,
        rotation_deg: f64,
        scale: f64,
    },
}

impl Entity {
    #[must_use]
    pub fn id(&self) -> &EntityId {
        match self {
            Self::Line { id, .. }
            | Self::Polyline { id, .. }
            | Self::Arc { id, .. }
            | Self::Circle { id, .. }
            | Self::Text { id, .. }
            | Self::Dimension { id, .. }
            | Self::BlockRef { id, .. } => id,
        }
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        match self {
            Self::Line { schema_version, .. }
            | Self::Polyline { schema_version, .. }
            | Self::Arc { schema_version, .. }
            | Self::Circle { schema_version, .. }
            | Self::Text { schema_version, .. }
            | Self::Dimension { schema_version, .. }
            | Self::BlockRef { schema_version, .. } => schema_version,
        }
    }

    #[must_use]
    pub fn layer(&self) -> &str {
        match self {
            Self::Line { layer, .. }
            | Self::Polyline { layer, .. }
            | Self::Arc { layer, .. }
            | Self::Circle { layer, .. }
            | Self::Text { layer, .. }
            | Self::Dimension { layer, .. }
            | Self::BlockRef { layer, .. } => layer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min: Point,
    pub max: Point,
}

impl BBox {
    pub fn from_points(points: &[Point]) -> Option<Self> {
        let mut iter = points.iter();
        let first = *iter.next()?;
        if !point_is_finite(first) {
            return None;
        }

        let mut bbox = Self {
            min: first,
            max: first,
        };
        for point in iter {
            if !point_is_finite(*point) {
                return None;
            }
            bbox.min[0] = bbox.min[0].min(point[0]);
            bbox.min[1] = bbox.min[1].min(point[1]);
            bbox.max[0] = bbox.max[0].max(point[0]);
            bbox.max[1] = bbox.max[1].max(point[1]);
        }
        Some(bbox)
    }

    pub fn from_center_radius(center: Point, radius: f64) -> Option<Self> {
        if !point_is_finite(center) || !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        Some(Self {
            min: [center[0] - radius, center[1] - radius],
            max: [center[0] + radius, center[1] + radius],
        })
    }

    #[must_use]
    pub fn width(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    #[must_use]
    pub fn height(&self) -> f64 {
        self.max[1] - self.min[1]
    }
}

pub fn entity_bbox(entity: &Entity) -> Option<BBox> {
    match entity {
        Entity::Line { p1, p2, .. } | Entity::Dimension { p1, p2, .. } => {
            BBox::from_points(&[*p1, *p2])
        }
        Entity::Polyline { points, .. } => BBox::from_points(points),
        Entity::Arc { center, radius, .. } | Entity::Circle { center, radius, .. } => {
            BBox::from_center_radius(*center, *radius)
        }
        Entity::Text { at, .. } | Entity::BlockRef { at, .. } => BBox::from_points(&[*at]),
    }
}

fn point_is_finite(point: Point) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityRecord {
    pub line: usize,
    pub entity: Entity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingSource {
    pub name: String,
    pub sheet: SheetConfig,
    pub entities: Vec<EntityRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSource {
    pub root: PathBuf,
    pub project: ProjectConfig,
    pub layers: LayerRules,
    pub styles: StyleRules,
    pub drawings: Vec<DrawingSource>,
}

pub fn load_project(root: impl AsRef<Path>) -> ModelResult<ProjectSource> {
    let root = root.as_ref();
    let project_path = root.join("cad.project.toml");
    let layers_path = root.join("rules/layers.toml");
    let styles_path = root.join("rules/styles.toml");

    let project: ProjectConfig = read_toml(&project_path)?;
    ensure_schema(&project_path, None, &project.schema_version)?;

    let layers = read_toml(&layers_path)?;
    let styles = read_toml(&styles_path)?;
    let drawings = read_drawings(root)?;

    Ok(ProjectSource {
        root: root.to_path_buf(),
        project,
        layers,
        styles,
        drawings,
    })
}

#[must_use]
pub fn format_decimal_mm(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let mut text = format!("{rounded:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        return "0".to_owned();
    }
    text
}

fn read_drawings(root: &Path) -> ModelResult<Vec<DrawingSource>> {
    let drawings_dir = root.join("drawings");
    let mut entries = fs::read_dir(&drawings_dir)
        .map_err(|source| ModelError::ListDir {
            path: drawings_dir.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ModelError::ListDir {
            path: drawings_dir.clone(),
            source,
        })?;

    entries.sort_by_key(|entry| entry.file_name());

    entries
        .into_iter()
        .filter_map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => Some(Ok(entry)),
            Ok(_) => None,
            Err(source) => Some(Err(ModelError::ListDir {
                path: drawings_dir.clone(),
                source,
            })),
        })
        .map(|entry| {
            let entry = entry?;
            let drawing_dir = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let sheet_path = drawing_dir.join("sheet.toml");
            let entities_path = drawing_dir.join("entities.ndjson");

            let sheet: SheetConfig = read_toml(&sheet_path)?;
            ensure_schema(&sheet_path, None, &sheet.schema_version)?;
            let entities = read_entities(&entities_path)?;

            Ok(DrawingSource {
                name,
                sheet,
                entities,
            })
        })
        .collect()
}

fn read_toml<T>(path: &Path) -> ModelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path).map_err(|source| ModelError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| ModelError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn read_entities(path: &Path) -> ModelResult<Vec<EntityRecord>> {
    let text = fs::read_to_string(path).map_err(|source| ModelError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    text.lines()
        .enumerate()
        .map(|(index, line)| {
            let line_number = index + 1;
            if line.trim().is_empty() {
                return Err(ModelError::EmptyNdjsonLine {
                    path: path.to_path_buf(),
                    line: line_number,
                });
            }

            let entity: Entity =
                serde_json::from_str(line).map_err(|source| ModelError::Ndjson {
                    path: path.to_path_buf(),
                    line: line_number,
                    source,
                })?;
            ensure_schema(path, Some(line_number), entity.schema_version())?;
            Ok(EntityRecord {
                line: line_number,
                entity,
            })
        })
        .collect()
}

fn ensure_schema(path: &Path, line: Option<usize>, found: &str) -> ModelResult<()> {
    if found == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    Err(ModelError::UnsupportedSchema {
        path: path.to_path_buf(),
        line,
        found: found.to_owned(),
        expected: CURRENT_SCHEMA_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-model");
    }

    #[test]
    fn loads_house_small_example() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");

        let project = load_project(root).expect("example project should load");

        assert_eq!(project.project.name, "house-small");
        assert_eq!(project.drawings.len(), 1);
        assert_eq!(project.drawings[0].name, "plan_1f");
        assert_eq!(project.drawings[0].entities.len(), 1);
        assert_eq!(project.drawings[0].entities[0].line, 1);
        assert_eq!(
            project.drawings[0].entities[0].entity.id().as_str(),
            "ent_01JZ0000000000000000000000"
        );
    }

    #[test]
    fn parses_all_mvp_entity_types() {
        let source = [
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[1.0,0.0]}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0.0,0.0],[1.0,0.0]],"closed":false}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"arc","layer":"0-1","center":[0.0,0.0],"radius":1.0,"start_deg":0.0,"end_deg":90.0}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000003","type":"circle","layer":"0-1","center":[0.0,0.0],"radius":1.0}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000004","type":"text","layer":"0-1","style":"note","at":[0.0,0.0],"rotation_deg":0.0,"value":"room"}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000005","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[1.0,0.0],"offset":100.0,"value":null}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000006","type":"block_ref","layer":"0-1","block":"door_910","at":[0.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
        ];

        for line in source {
            let entity: Entity = serde_json::from_str(line).expect("entity should parse");
            assert_eq!(entity.schema_version(), CURRENT_SCHEMA_VERSION);
        }
    }

    #[test]
    fn rejects_invalid_json_line() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            "{\"schema_version\":\"0.1\"",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("invalid JSON should fail");

        assert!(matches!(error, ModelError::Ndjson { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_toml() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = ",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("invalid TOML should fail");

        assert!(matches!(error, ModelError::Toml { .. }));
    }

    #[test]
    fn rejects_unknown_project_schema() {
        let temp = minimal_project();
        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"9.9\"\nname = \"bad\"\n",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown schema should fail");

        assert!(matches!(
            error,
            ModelError::UnsupportedSchema { line: None, .. }
        ));
    }

    #[test]
    fn rejects_unknown_entity_schema() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"9.9","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown entity schema should fail");

        assert!(matches!(
            error,
            ModelError::UnsupportedSchema { line: Some(1), .. }
        ));
    }

    #[test]
    fn rejects_unknown_entity_type() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"spline","layer":"0-1"}"#,
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown entity type should fail");

        assert!(matches!(error, ModelError::Ndjson { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_entity_id() {
        let error = EntityId::parse("bad_01JZ0000000000000000000000")
            .expect_err("non entity prefix should fail");

        assert!(matches!(error, ModelError::InvalidEntityId { .. }));
    }

    #[test]
    fn fixes_decimal_format_snapshot() {
        insta::assert_snapshot!(
            [
                format_decimal_mm(1.2344),
                format_decimal_mm(1.2345),
                format_decimal_mm(10.0),
                format_decimal_mm(-0.0004),
            ]
            .join("\n"),
            @"
1.234
1.235
10
0
"
        );
    }

    #[test]
    fn computes_entity_bbox() {
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[-1.0,2.0],"p2":[3.0,-4.0]}"#,
        )
        .expect("line should parse");

        let bbox = entity_bbox(&entity).expect("line should have bbox");

        assert_eq!(bbox.min, [-1.0, -4.0]);
        assert_eq!(bbox.max, [3.0, 2.0]);
    }

    fn minimal_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"fixture\"\n",
        )
        .expect("project TOML should be writable");
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
        )
        .expect("entities NDJSON should be writable");

        temp
    }
}
