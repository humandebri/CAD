use super::*;
use cad_model::{
    BlockDefinition, BlockDefinitionConfig, ColorDef, EntityId, EntityRecord, LayerDef,
    LineTypeDef, PenStyleDef, TextStyleDef,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use ulid::Ulid;

pub struct DxfImport {
    pub source: Option<ProjectSource>,
    pub report: ExchangeReport,
    _directory: tempfile::TempDir,
}
struct Context<'a> {
    source: &'a mut ProjectSource,
    report: &'a mut ExchangeReport,
    layers: BTreeMap<String, String>,
    blocks: BTreeMap<String, String>,
    line_types: BTreeMap<String, String>,
    scale: f64,
}

/// Parsing/conversion failures are represented in the compatibility report.
pub fn import(bytes: &[u8], name: &str, unit_scale_mm: Option<f64>) -> Result<DxfImport> {
    match import_impl(bytes, name, unit_scale_mm) {
        Ok(result) => Ok(result),
        Err(error) => {
            let mut report = ExchangeReport::new();
            report.format = "dxf-import".into();
            report.status = "blocked".into();
            report.source_blake3 = Some(blake3::hash(bytes).to_hex().to_string());
            let _ = scan_records(bytes, &mut report);
            report.block(None, "conversion_failed", error.to_string());
            Ok(DxfImport {
                source: None,
                report,
                _directory: tempfile::tempdir().map_err(invalid)?,
            })
        }
    }
}

fn import_impl(bytes: &[u8], name: &str, unit_scale_mm: Option<f64>) -> Result<DxfImport> {
    let directory = tempfile::tempdir().map_err(invalid)?;
    let mut report = ExchangeReport::new();
    report.source_blake3 = Some(blake3::hash(bytes).to_hex().to_string());
    report.format = "dxf-import".into();
    let (version, code_page) = scan_records(bytes, &mut report)?;
    if !report.blockers.is_empty() {
        report.status = "blocked".into();
        return Ok(DxfImport {
            source: None,
            report,
            _directory: directory,
        });
    }
    let encoding = if version.as_str() >= "AC1021" {
        encoding_rs::UTF_8
    } else {
        match code_page.as_str() {
            "ANSI_932" => encoding_rs::SHIFT_JIS,
            "ANSI_1252" | "" => encoding_rs::WINDOWS_1252,
            _ => {
                return Err(invalid(format!(
                    "DXF code page {code_page:?} requires explicit encoding support"
                )));
            }
        }
    };
    let drawing =
        Drawing::load_with_encoding(&mut Cursor::new(bytes), encoding).map_err(invalid)?;
    let scale = if let Some(scale) = unit_scale_mm {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(invalid("unit scale must be finite and positive"));
        }
        report.warning(
            None,
            "units_override",
            format!("Input unit scale explicitly overridden as {scale} mm per DXF unit."),
        );
        scale
    } else {
        match drawing.header.default_drawing_units {
            Units::Millimeters => 1.0,
            Units::Centimeters => 10.0,
            Units::Meters => 1000.0,
            Units::Inches => 25.4,
            Units::Feet => 304.8,
            Units::Unitless => {
                report.block(
                    None,
                    "unitless_input",
                    "DXF has no specified length unit; provide --unit-mm explicitly.",
                );
                0.0
            }
            _ => {
                report.block(
                    None,
                    "unsupported_units",
                    "This DXF unit is not yet supported; provide --unit-mm explicitly.",
                );
                0.0
            }
        }
    };
    if !report.blockers.is_empty() {
        report.status = "blocked".into();
        return Ok(DxfImport {
            source: None,
            report,
            _directory: directory,
        });
    }
    let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
        parent_dir: directory.path().display().to_string(),
        folder_name: "project".into(),
        project_name: name.into(),
        drawing: "imported".into(),
        paper: "A3".into(),
        orientation: cad_model::SheetOrientation::Landscape,
        scale_denominator: 100,
    })
    .map_err(invalid)?;
    let mut source = cad_model::load_project(&created.project_path).map_err(invalid)?;
    let mut context = Context {
        source: &mut source,
        report: &mut report,
        layers: BTreeMap::new(),
        blocks: BTreeMap::new(),
        line_types: BTreeMap::new(),
        scale,
    };
    for (index, line_type) in drawing.line_types().enumerate() {
        let lengths = &line_type.dash_dot_space_lengths;
        let simple = line_type
            .complex_line_type_element_types
            .iter()
            .all(|v| *v == 0)
            && (lengths.is_empty()
                || (lengths.len() % 2 == 0
                    && lengths.iter().enumerate().all(|(i, v)| {
                        v.is_finite() && if i % 2 == 0 { *v > 0.0 } else { *v < 0.0 }
                    })));
        let id = format!("dxf_line_type_{index:04}");
        let dash = if simple {
            lengths
                .iter()
                .map(|v| v.abs() * scale * drawing.header.line_type_scale / 100.0)
                .collect()
        } else {
            context.report.warning(None, "linetype_approximation", format!("DXF line type {:?} contains complex, dot, invalid or phase-shifted elements and uses a solid approximation.",line_type.name));
            Vec::new()
        };
        context
            .source
            .styles
            .line_types
            .insert(id.clone(), LineTypeDef { dash });
        context.line_types.insert(line_type.name.clone(), id);
    }
    for layer in drawing.layers() {
        let id = context.layer(&layer.name)?;
        let color = indexed_color(&layer.color, context.report);
        let color_id = format!("{id}_color");
        context.source.styles.colors.insert(
            color_id.clone(),
            ColorDef {
                rgb: color,
                print_rgb: None,
                print_width: 0.25,
            },
        );
        let definition = context.source.layers.layers.get_mut(&id).unwrap();
        definition.color = color_id;
        definition.visible = layer.is_layer_on;
        definition.printable = layer.is_layer_plotted;
        definition.line_width = if layer.line_weight.raw_value() > 0 {
            layer.line_weight.raw_value() as f64 / 100.0
        } else {
            0.25
        };
        if let Some(id) = context.line_types.get(&layer.line_type_name) {
            context
                .source
                .layers
                .layers
                .get_mut(&context.layers[&layer.name])
                .unwrap()
                .line_type = id.clone();
        } else if layer.line_type_name != "CONTINUOUS" {
            context.report.warning(
                None,
                "missing_linetype",
                format!(
                    "Layer {:?} refers to absent line type {:?}; using solid.",
                    layer.name, layer.line_type_name
                ),
            );
        }
    }
    let definitions: Vec<_> = drawing
        .blocks()
        .filter(|b| !b.name.starts_with('*'))
        .collect();
    for (index, block) in definitions.iter().enumerate() {
        context
            .blocks
            .insert(block.name.clone(), format!("dxf_block_{index:04}"));
    }
    for block in definitions {
        if block.is_xref() || block.is_xref_overlay() {
            context.report.block(None,"external_block","External DXF block references require the linked file; no canonical project is published.");
            continue;
        }
        let mut entities = Vec::new();
        for entity in &block.entities {
            if let Some(entity) = context.entity(entity)? {
                entities.push(EntityRecord {
                    line: entities.len() + 1,
                    entity,
                });
            }
        }
        let id = context.blocks[&block.name].clone();
        context.source.blocks.insert(
            id.clone(),
            BlockDefinition {
                id,
                config: BlockDefinitionConfig {
                    schema_version: cad_model::CURRENT_SCHEMA_VERSION.into(),
                    name: block.name.clone(),
                    base_point: xy(&block.base_point, scale)?,
                },
                entities,
            },
        );
    }
    let mut entities = Vec::new();
    for entity in drawing.entities() {
        if let Some(entity) = context.entity(entity)? {
            entities.push(EntityRecord {
                line: entities.len() + 1,
                entity,
            });
        }
    }
    context.source.drawings[0].entities = entities;
    context.report.warning(None,"new_project_layout","DXF model space is imported into an A3 landscape layout at 1:100. Original paper layouts, font metrics, handles and source IDs are not retained.");
    if context.report.blockers.is_empty() {
        write_source(context.source)?;
        let check = cad_check::check_project(&context.source.root);
        if !check.is_ok() {
            context.report.block(
                None,
                "canonical_check",
                format!("Imported candidate failed CAD validation: {check:?}"),
            );
        }
    }
    let source = if report.blockers.is_empty() {
        Some(source)
    } else {
        report.status = "blocked".into();
        None
    };
    Ok(DxfImport {
        source,
        report,
        _directory: directory,
    })
}

fn xy(point: &DxfPoint, scale: f64) -> Result<Point> {
    if ![point.x, point.y, point.z].iter().all(|v| v.is_finite()) || point.z.abs() > 1e-8 {
        return Err(invalid(
            "Nonplanar or nonfinite DXF coordinates require 3D conversion; they are not flattened.",
        ));
    }
    Ok([point.x * scale, point.y * scale])
}
fn planar(normal: &Vector) -> Result<()> {
    if ![normal.x, normal.y, normal.z].iter().all(|v| v.is_finite())
        || normal.x.abs() > 1e-8
        || normal.y.abs() > 1e-8
        || (normal.z - 1.0).abs() > 1e-8
    {
        return Err(invalid(
            "Nonstandard DXF extrusion coordinates are not projected or guessed.",
        ));
    }
    Ok(())
}
fn indexed_color(color: &dxf::Color, report: &mut ExchangeReport) -> String {
    match color.index().unwrap_or(7) {
        1 => "#FF0000",
        2 => "#FFFF00",
        3 => "#00FF00",
        4 => "#00FFFF",
        5 => "#0000FF",
        6 => "#FF00FF",
        7 => "#000000",
        8 => "#808080",
        9 => "#C0C0C0",
        _ => {
            report.warning(
                None,
                "indexed_color_approximation",
                "Extended ACI palette color uses black; verify color after import.",
            );
            "#000000"
        }
    }
    .into()
}
impl Context<'_> {
    fn layer(&mut self, name: &str) -> Result<String> {
        if let Some(id) = self.layers.get(name) {
            return Ok(id.clone());
        }
        let id = format!("dxf_layer_{:04}", self.layers.len());
        let template = self.source.layers.layers["0-1"].clone();
        self.source.layers.layers.insert(
            id.clone(),
            LayerDef {
                name: name.into(),
                group: None,
                ..template
            },
        );
        self.layers.insert(name.into(), id.clone());
        if self.layers.len() == 1 {
            self.source.layers.active_layer = Some(id.clone());
        }
        Ok(id)
    }
    fn entity(&mut self, input: &dx::Entity) -> Result<Option<Entity>> {
        if input.common.is_in_paper_space {
            self.report.block(
                None,
                "paper_space_entity",
                "Paper-space entities need explicit viewport/layout conversion.",
            );
            return Ok(None);
        }
        if !input.common.elevation.is_finite() || input.common.elevation.abs() > 1e-8 {
            return Err(invalid("DXF elevation cannot be flattened implicitly"));
        }
        let layer = self.layer(&input.common.layer)?;
        let scale = self.scale;
        let mut value = match &input.specific {
            dx::EntityType::Line(line) => {
                planar(&line.extrusion_direction)?;
                if line.thickness != 0.0 {
                    return Err(invalid("3D line thickness is unsupported"));
                }
                json!({"type":"line","p1":xy(&line.p1,scale)?,"p2":xy(&line.p2,scale)?})
            }
            dx::EntityType::Circle(circle) => {
                planar(&circle.normal)?;
                if circle.thickness != 0.0 {
                    return Err(invalid("3D circle thickness is unsupported"));
                }
                json!({"type":"circle","center":xy(&circle.center,scale)?,"radius":circle.radius*scale})
            }
            dx::EntityType::Arc(arc) => {
                planar(&arc.normal)?;
                if arc.thickness != 0.0 {
                    return Err(invalid("3D arc thickness is unsupported"));
                }
                let sweep = (arc.end_angle - arc.start_angle).rem_euclid(360.0);
                if sweep <= 1e-9 {
                    return Err(invalid("DXF ARC with identical angles is ambiguous"));
                }
                json!({"type":"arc","center":xy(&arc.center,scale)?,"radius":arc.radius*scale,"start_deg":arc.start_angle,"end_deg":arc.start_angle+sweep})
            }
            dx::EntityType::Ellipse(ellipse) => {
                planar(&ellipse.normal)?;
                if ellipse.major_axis.z.abs() > 1e-8 {
                    return Err(invalid("3D ellipse major axis is unsupported"));
                }
                let radius = ellipse.major_axis.x.hypot(ellipse.major_axis.y) * scale;
                let span = ellipse.end_parameter - ellipse.start_parameter;
                let span = if span.abs() >= TAU - 1e-9 {
                    TAU
                } else {
                    span.rem_euclid(TAU)
                };
                json!({"type":"ellipse","center":xy(&ellipse.center,scale)?,"radius_x":radius,"radius_y":radius*ellipse.minor_axis_ratio,"rotation_deg":ellipse.major_axis.y.atan2(ellipse.major_axis.x).to_degrees(),"start_deg":ellipse.start_parameter.to_degrees(),"end_deg":(ellipse.start_parameter+span).to_degrees()})
            }
            dx::EntityType::LwPolyline(poly) => {
                planar(&poly.extrusion_direction)?;
                if poly.thickness != 0.0
                    || poly.constant_width != 0.0
                    || poly
                        .vertices
                        .iter()
                        .any(|v| v.bulge != 0.0 || v.starting_width != 0.0 || v.ending_width != 0.0)
                {
                    self.report.block(
                        None,
                        "polyline_curves_width",
                        "Bulge and variable-width polylines require curve/stroke conversion.",
                    );
                    return Ok(None);
                }
                let mut points: Vec<_> = poly
                    .vertices
                    .iter()
                    .map(|v| [v.x * scale, v.y * scale])
                    .collect();
                if poly.is_closed() && points.first() != points.last() && !points.is_empty() {
                    points.push(points[0]);
                }
                json!({"type":"polyline","points":points,"closed":poly.is_closed()})
            }
            dx::EntityType::Polyline(poly) => {
                planar(&poly.normal)?;
                if xy(&poly.location, scale)? != [0.0, 0.0] {
                    return Err(invalid("Legacy POLYLINE origin must be zero"));
                }
                if poly.flags & !1 != 0
                    || poly.thickness != 0.0
                    || poly.default_starting_width != 0.0
                    || poly.default_ending_width != 0.0
                    || poly.vertices().any(|v| {
                        v.flags != 0
                            || v.bulge != 0.0
                            || v.starting_width != 0.0
                            || v.ending_width != 0.0
                    })
                {
                    self.report.block(None, "legacy_polyline_transform",
                        "Curve-fitted, mesh, bulged or variable-width POLYLINE requires explicit conversion.");
                    return Ok(None);
                }
                let mut points: Vec<_> = poly
                    .vertices()
                    .map(|v| xy(&v.location, scale))
                    .collect::<Result<_>>()?;
                if poly.is_closed() && points.first() != points.last() && !points.is_empty() {
                    points.push(points[0]);
                }
                json!({"type":"polyline","points":points,"closed":poly.is_closed()})
            }
            dx::EntityType::ModelPoint(point) => {
                planar(&point.extrusion_direction)?;
                if point.thickness != 0.0 {
                    return Err(invalid("3D point thickness is unsupported"));
                }
                json!({"type":"point","at":xy(&point.location,scale)?,"temporary":false})
            }
            dx::EntityType::Text(text) => {
                planar(&text.normal)?;
                if text.thickness != 0.0
                    || text.oblique_angle != 0.0
                    || text.is_text_backwards()
                    || text.relative_x_scale_factor <= 0.0
                {
                    self.report.block(None,"text_transform","Oblique, backward or nonpositive-width text requires explicit text conversion.");
                    return Ok(None);
                }
                let align = match text.horizontal_text_justification {
                    dxf::enums::HorizontalTextJustification::Left => cad_model::TextAlign::Left,
                    dxf::enums::HorizontalTextJustification::Center => cad_model::TextAlign::Center,
                    dxf::enums::HorizontalTextJustification::Right => cad_model::TextAlign::Right,
                    _ => {
                        self.report.block(
                            None,
                            "text_alignment",
                            "Fitted/aligned DXF text is not yet supported.",
                        );
                        return Ok(None);
                    }
                };
                // Canonical text uses a baseline anchor. DXF uses the second
                // alignment point whenever either justification is non-default.
                let vertical_fraction = match text.vertical_text_justification {
                    dxf::enums::VerticalTextJustification::Baseline
                    | dxf::enums::VerticalTextJustification::Bottom => 0.0,
                    dxf::enums::VerticalTextJustification::Middle => 0.5,
                    dxf::enums::VerticalTextJustification::Top => 1.0,
                };
                let id = format!("dxf_text_{:04}", self.source.styles.text_styles.len());
                let font = "M+ 1p".to_owned();
                self.source.styles.text_styles.insert(
                    id.clone(),
                    TextStyleDef {
                        font_family: font,
                        height: text.text_height * scale,
                        width: text.text_height * text.relative_x_scale_factor * scale,
                        spacing: 0.0,
                        align,
                    },
                );
                let anchor = if text.horizontal_text_justification
                    == dxf::enums::HorizontalTextJustification::Left
                    && text.vertical_text_justification
                        == dxf::enums::VerticalTextJustification::Baseline
                {
                    &text.location
                } else {
                    &text.second_alignment_point
                };
                let mut at = xy(anchor, scale)?;
                if text.vertical_text_justification
                    != dxf::enums::VerticalTextJustification::Baseline
                {
                    let direction = if text.is_text_upside_down() {
                        -1.0
                    } else {
                        1.0
                    };
                    let shift = -vertical_fraction * text.text_height * scale * direction;
                    let angle = text.rotation.to_radians();
                    at[0] -= shift * angle.sin();
                    at[1] += shift * angle.cos();
                    self.report.warning(None, "text_vertical_alignment_approximation",
                        format!("DXF {:?} text alignment is converted to a baseline using nominal height; substituted font cap-height and descender metrics require visual comparison.", text.vertical_text_justification));
                }
                self.report.warning(None,"text_font_substitution",format!("DXF text style {:?} is mapped to M+ 1p; font metrics require visual comparison.",text.text_style_name));
                json!({"type":"text","at":at,"rotation_deg":text.rotation,"mirror_y":text.is_text_upside_down(),"style":id,"value":text.value})
            }
            dx::EntityType::Insert(insert) => {
                planar(&insert.extrusion_direction)?;
                if insert.row_count != 1
                    || insert.column_count != 1
                    || insert.attributes().next().is_some()
                    || (insert.x_scale_factor.abs() - insert.y_scale_factor.abs()).abs() > 1e-8
                    || insert.x_scale_factor == 0.0
                {
                    self.report.block(None,"insert_transform","Array, attributed or nonuniform DXF block inserts require explicit expansion.");
                    return Ok(None);
                }
                let Some(block) = self.blocks.get(&insert.name) else {
                    self.report.block(
                        None,
                        "missing_block",
                        format!("DXF block {:?} is absent or unsupported.", insert.name),
                    );
                    return Ok(None);
                };
                json!({"type":"block_ref","block":block,"at":xy(&insert.location,scale)?,"rotation_deg":insert.rotation,"scale":insert.x_scale_factor.abs(),"mirror_x":insert.x_scale_factor<0.0,"mirror_y":insert.y_scale_factor<0.0})
            }
            dx::EntityType::Solid(solid) => {
                planar(&solid.extrusion_direction)?;
                if solid.thickness != 0.0 {
                    return Err(invalid("3D SOLID thickness is unsupported"));
                }
                let a = xy(&solid.first_corner, scale)?;
                let b = xy(&solid.second_corner, scale)?;
                let c = xy(&solid.third_corner, scale)?;
                let d = xy(&solid.fourth_corner, scale)?;
                let points = if c == d {
                    vec![a, b, c]
                } else {
                    vec![a, b, d, c]
                };
                json!({"type":"solid","points":points,"fill":"black"})
            }
            _ => {
                self.report.block(
                    None,
                    "unsupported_entity",
                    format!("Unsupported decoded DXF entity: {:?}", input.specific),
                );
                return Ok(None);
            }
        };
        let color = if input.common.color_24_bit != 0 {
            format!("#{:06X}", input.common.color_24_bit & 0xFFFFFF)
        } else if input.common.color.is_by_layer() {
            self.source.styles.colors[&self.source.layers.layers[&layer].color]
                .rgb
                .clone()
        } else {
            if input.common.color.is_by_block() {
                self.report.warning(
                    None,
                    "byblock_color",
                    "BYBLOCK color is mapped to the entity layer color.",
                );
                self.source.styles.colors[&self.source.layers.layers[&layer].color]
                    .rgb
                    .clone()
            } else {
                indexed_color(&input.common.color, self.report)
            }
        };
        let color_id = format!("dxf_color_{:04}", self.source.styles.colors.len());
        self.source.styles.colors.insert(
            color_id.clone(),
            ColorDef {
                rgb: color,
                print_rgb: None,
                print_width: 0.25,
            },
        );
        let pen = format!("dxf_pen_{:04}", self.source.styles.pens.len());
        let width = if input.common.lineweight_enum_value > 0 {
            input.common.lineweight_enum_value as f64 / 100.0
        } else {
            self.source.layers.layers[&layer].line_width
        };
        let mut line_type = if input.common.line_type_name == "BYLAYER"
            || input.common.line_type_name == "BYBLOCK"
        {
            if input.common.line_type_name == "BYBLOCK" {
                self.report.warning(
                    None,
                    "byblock_linetype",
                    "BYBLOCK line type is mapped to the entity layer line type.",
                );
            }
            self.source.layers.layers[&layer].line_type.clone()
        } else if let Some(id) = self.line_types.get(&input.common.line_type_name) {
            id.clone()
        } else {
            self.report.warning(
                None,
                "missing_entity_linetype",
                format!(
                    "Entity line type {:?} is absent; using the layer line type.",
                    input.common.line_type_name
                ),
            );
            self.source.layers.layers[&layer].line_type.clone()
        };
        if !input.common.line_type_scale.is_finite() || input.common.line_type_scale <= 0.0 {
            return Err(invalid(
                "DXF entity line type scale must be finite and positive",
            ));
        }
        if input.common.line_type_scale != 1.0 {
            let dash = self.source.styles.line_types[&line_type]
                .dash
                .iter()
                .map(|v| v * input.common.line_type_scale)
                .collect();
            line_type = format!("{pen}_line_type");
            self.source
                .styles
                .line_types
                .insert(line_type.clone(), LineTypeDef { dash });
        }
        self.source.styles.pens.insert(
            pen.clone(),
            PenStyleDef {
                color: color_id.clone(),
                line_type,
                line_width: width,
            },
        );
        if !input.common.is_visible || input.common.transparency != 0 {
            self.report.warning(None,"entity_visibility","Per-entity visibility or transparency is not retained; inspect the imported geometry.");
        }
        value["schema_version"] = json!(cad_model::CURRENT_SCHEMA_VERSION);
        value["id"] = json!(format!("ent_{}", Ulid::new()));
        value["layer"] = json!(layer);
        value["pen"] = json!(pen);
        if value["type"] == "solid" {
            value["fill"] = json!(color_id);
        }
        let entity: Entity = serde_json::from_value(value).map_err(invalid)?;
        let _: &EntityId = entity.id();
        Ok(Some(entity))
    }
}

fn write_source(source: &ProjectSource) -> Result<()> {
    fn write(path: &Path, bytes: &[u8]) -> Result<()> {
        fs::create_dir_all(path.parent().unwrap()).map_err(invalid)?;
        fs::write(path, bytes).map_err(invalid)
    }
    write(
        &source.root.join("rules/layers.toml"),
        toml::to_string_pretty(&source.layers)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    write(
        &source.root.join("rules/styles.toml"),
        toml::to_string_pretty(&source.styles)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    let ndjson = |entities: &[EntityRecord]| -> Result<String> {
        entities
            .iter()
            .map(|r| {
                serde_json::to_string(&r.entity)
                    .map(|s| s + "\n")
                    .map_err(invalid)
            })
            .collect()
    };
    write(
        &source.root.join("drawings/imported/entities.ndjson"),
        ndjson(&source.drawings[0].entities)?.as_bytes(),
    )?;
    for (id, block) in &source.blocks {
        write(
            &source.root.join("blocks").join(id).join("definition.toml"),
            toml::to_string_pretty(&block.config)
                .map_err(invalid)?
                .as_bytes(),
        )?;
        write(
            &source.root.join("blocks").join(id).join("entities.ndjson"),
            ndjson(&block.entities)?.as_bytes(),
        )?;
    }
    Ok(())
}

fn scan_records(bytes: &[u8], report: &mut ExchangeReport) -> Result<(String, String)> {
    let lines: Vec<_> = bytes.split(|b| *b == b'\n').collect();
    let mut section = String::new();
    let mut expect_section = false;
    let mut variable = String::new();
    let mut version = String::new();
    let mut page = String::new();
    let mut eof = false;
    let supported: BTreeSet<_> = [
        "LINE",
        "CIRCLE",
        "ARC",
        "ELLIPSE",
        "TEXT",
        "INSERT",
        "POINT",
        "LWPOLYLINE",
        "POLYLINE",
        "VERTEX",
        "SEQEND",
        "SOLID",
    ]
    .into_iter()
    .collect();
    let mut index = 0;
    while index + 1 < lines.len() {
        let code = std::str::from_utf8(lines[index])
            .map_err(invalid)?
            .trim()
            .parse::<i32>()
            .map_err(|_| {
                invalid(
                    "Only ASCII tagged DXF is supported; malformed or binary DXF is not guessed.",
                )
            })?;
        let value = String::from_utf8_lossy(lines[index + 1]).trim().to_owned();
        index += 2;
        if code == 0 {
            match value.as_str() {
                "SECTION" => expect_section = true,
                "ENDSEC" => section.clear(),
                "EOF" => {
                    eof = true;
                    break;
                }
                name if (section == "ENTITIES" || section == "BLOCKS")
                    && name != "BLOCK"
                    && name != "ENDBLK" =>
                {
                    *report.record_counts.entry(name.into()).or_default() += 1;
                    if !supported.contains(name) {
                        report.block(
                            None,
                            "unsupported_record",
                            format!("DXF record {name} has no supported canonical conversion."),
                        );
                    }
                }
                _ => {}
            }
        } else if expect_section && code == 2 {
            section = value.clone();
            expect_section = false;
        } else if section == "HEADER" && code == 9 {
            variable = value.clone();
        } else if section == "HEADER" {
            if variable == "$ACADVER" && code == 1 {
                version = value.clone();
            }
            if variable == "$DWGCODEPAGE" && code == 3 {
                page = value;
            }
        }
    }
    if !eof {
        return Err(invalid("DXF EOF marker is missing"));
    }
    Ok((version, page))
}
