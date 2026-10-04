//! JWS is an import boundary, without JWW preservation or lossless claims.
use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct SymbolImportReport {
    pub schema_version: String,
    pub status: String,
    pub source_hash: String,
    pub header: Option<cad_jww_codec::SymbolHeader>,
    /// Explicitly chosen model millimetres per stored coordinate unit.
    pub coordinate_scale: f64,
    pub placement_origin_mm: Option<[f64; 2]>,
    pub drawing_name: Option<String>,
    pub supported_entities: usize,
    pub warnings: Vec<ImportWarning>,
    pub blockers: Vec<String>,
    pub check: Option<cad_check::CheckReport>,
    pub exact_round_trip: bool,
}

/// Prepares a new canonical candidate. The caller owns publication and must
/// retain this report on failure. Palette, sheet and fonts are replacements:
/// JWS does not carry the JWW drawing's full environment.
pub fn prepare_symbol_import(
    data: &[u8],
    name: &str,
    coordinate_scale: f64,
    write_dir: &Path,
) -> ImportResult<SymbolImportReport> {
    let mut report = SymbolImportReport {
        schema_version: "cad-jws-import/1".into(),
        status: "blocked".into(),
        source_hash: blake3::hash(data).to_hex().to_string(),
        header: None,
        coordinate_scale,
        placement_origin_mm: None,
        drawing_name: None,
        supported_entities: 0,
        warnings: Vec::new(),
        blockers: Vec::new(),
        check: None,
        exact_round_trip: false,
    };
    if !coordinate_scale.is_finite() || coordinate_scale <= 0. {
        report
            .blockers
            .push("coordinate scale must be finite and positive".into());
        return Ok(report);
    }
    if write_dir.exists() {
        let mut entries = fs::read_dir(write_dir).map_err(|source| ImportError::Read {
            path: write_dir.to_path_buf(),
            source,
        })?;
        if entries.next().is_some() {
            report
                .blockers
                .push("symbol candidate directory must be empty".into());
            return Ok(report);
        }
    }
    let symbol = match cad_jww_codec::read_symbol(data) {
        Ok(symbol) => symbol,
        Err(error) => {
            report.blockers.push(error.to_string());
            return Ok(report);
        }
    };
    report.placement_origin_mm = Some(symbol.header.origin.map(|v| v * coordinate_scale));
    report.header = Some(symbol.header.clone());
    if symbol
        .header
        .origin
        .iter()
        .chain(symbol.header.bounds.iter())
        .any(|v| !(v * coordinate_scale).is_finite())
    {
        report
            .blockers
            .push("coordinate scaling overflows the symbol origin or bounds".into());
        return Ok(report);
    }
    let defaults = cad_jww_codec::Header::default();
    let transform = Transform2D {
        a: coordinate_scale,
        b: 0.,
        c: 0.,
        d: coordinate_scale,
        tx: 0.,
        ty: 0.,
    };
    let document = JwwDocument {
        header: cad_jww_codec::DecodedHeader {
            version: symbol.header.version,
            memo: String::new(),
            paper_size: 3,
            write_layer_group: 0,
            layer_groups: std::array::from_fn(|group| cad_jww_codec::DecodedLayerGroup {
                name: format!("JWS {group:X}"),
                scale: coordinate_scale,
                state: 2,
                write_layer: 0,
                protect: 0,
                layers: std::array::from_fn(|layer| cad_jww_codec::DecodedLayer {
                    name: format!("{group:X}-{layer:X}"),
                    state: 2,
                    protect: 0,
                }),
            }),
            screen_pen_colors: defaults.screen_pen_colors,
            screen_pen_widths: defaults.screen_pen_widths,
            print_pen_colors: defaults.print_pen_colors,
            print_pen_widths: defaults.print_pen_widths,
            print_point_radii: defaults.print_point_radii,
        },
        entities: symbol
            .entities
            .iter()
            .map(|e| scaled(e, &transform))
            .collect(),
        block_defs: symbol
            .block_defs
            .iter()
            .map(|def| BlockDef {
                entities: def.entities.iter().map(|e| scaled(e, &transform)).collect(),
                ..def.clone()
            })
            .collect(),
    };
    // Flatten also validates every referenced definition's geometry and avoids
    // silently dropping child warnings in the older JWW preserve converter.
    let converted = match convert_entities_with_mode(
        &document,
        ConversionLimits::default(),
        BlockMode::Flatten,
    ) {
        Ok(converted) => converted,
        Err(error) => {
            report.blockers.push(error.to_string());
            return Ok(report);
        }
    };
    report.warnings = converted.warnings.clone();
    for warning in &mut report.warnings {
        warning.message = warning.message.replace(
            "; original record retained",
            "; original JWS is not embedded in this project",
        );
    }
    for warning in &report.warnings {
        if matches!(
            warning.code.as_str(),
            "geometry_skipped" | "unresolved_block" | "unsupported_scaled_curve" | "block_cycle"
        ) {
            report.blockers.push(warning.message.clone());
        }
    }
    if converted.entities.is_empty() {
        report
            .blockers
            .push("symbol has no editable geometry".into());
    }
    if !report.blockers.is_empty() {
        return Ok(report);
    }
    let (_, drawing_name, converted) = write_canonical_project(
        Path::new(name),
        write_dir,
        &document,
        converted,
        BlockMode::Flatten,
        coordinate_scale * 10.0,
    )?;
    report.supported_entities = converted.entities.len();
    report.drawing_name = Some(drawing_name);
    report.warnings = converted.warnings;
    for warning in &mut report.warnings {
        warning.message = warning.message.replace(
            "; original record retained",
            "; original JWS is not embedded in this project",
        );
    }
    report.warnings.push(ImportWarning {
        code:"jws_environment_replaced".into(), record_type:"header".into(),
        message:"JWS palette, layer names, print widths, fonts and sheet environment are not supplied; default palette, numbered layers, Hiragino Sans and A3 are used. Blocks are expanded. No exact JWS or JWW round trip is claimed.".into(),
    });
    report.warnings.push(ImportWarning {
        code:"explicit_coordinate_scale".into(), record_type:"header".into(),
        message:format!("All geometry and text lengths use the explicit factor {coordinate_scale} mm per stored unit; original group scales and placement origin are retained in this report. Mixed-scale reinterpretation is not automatic."),
    });
    let check = cad_check::check_project(write_dir);
    if !check.is_ok() {
        report
            .blockers
            .push("canonical candidate failed CAD validation".into());
    }
    report.check = Some(check);
    if report.blockers.is_empty() {
        report.status = "converted".into();
    }
    Ok(report)
}

fn scaled(entity: &JwwEntity, transform: &Transform2D) -> JwwEntity {
    match entity {
        JwwEntity::Line(v) => JwwEntity::Line(transform_line(v, transform)),
        JwwEntity::Arc(v) => JwwEntity::Arc(transform_arc(v, transform)),
        JwwEntity::Point(v) => JwwEntity::Point(transform_point_entity(v, transform)),
        JwwEntity::Text(v) => JwwEntity::Text(transform_text(v, transform)),
        JwwEntity::Solid(v) => JwwEntity::Solid(transform_solid(v, transform)),
        JwwEntity::CircleSolid(v) => JwwEntity::CircleSolid(transform_circle_solid(v, transform)),
        JwwEntity::Dimension(v) => JwwEntity::Dimension(transform_dimension(v, transform)),
        JwwEntity::Block(v) => JwwEntity::Block(Block {
            ref_x: v.ref_x * transform.a,
            ref_y: v.ref_y * transform.a,
            ..v.clone()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol() -> Vec<u8> {
        let document = cad_jww_codec::Document {
            header: cad_jww_codec::Header::default(),
            records: vec![cad_jww_codec::Record::Line {
                base: cad_jww_codec::Base::default(),
                p1: [1., 2.],
                p2: [11., 22.],
            }],
            blocks: vec![],
        };
        let jww = cad_jww_codec::write_document(&document).unwrap();
        let offset = jww
            .windows(14)
            .position(|b| b.ends_with(b"CDataSen"))
            .unwrap()
            - 2;
        let mut data = b"JwsData.".to_vec();
        data.extend([b'.'; 192]);
        data.extend(600u32.to_le_bytes());
        for value in [3f64, 4.] {
            data.extend(value.to_le_bytes());
        }
        for _ in 0..16 {
            data.extend(100f64.to_le_bytes());
        }
        data.extend([0; 72]);
        for value in [1f64, 2., 11., 22.] {
            data.extend(value.to_le_bytes());
        }
        data.extend(&jww[offset..jww.len() - 4]);
        data.extend([0; 2]);
        data
    }

    #[test]
    fn explicit_units_checked_candidate_without_fake_preservation() {
        let temp = tempfile::tempdir().unwrap();
        let report = prepare_symbol_import(&symbol(), "sample.jws", 100., temp.path()).unwrap();
        assert_eq!(report.status, "converted");
        assert!(report.check.unwrap().is_ok());
        assert!(!report.exact_round_trip);
        assert_eq!(report.placement_origin_mm, Some([300., 400.]));
        assert!(!temp.path().join("interop").exists());
        let manifest = cad_model::source_manifest(temp.path()).unwrap();
        assert_eq!(
            prepare_symbol_import(&symbol(), "sample.jws", 100., temp.path())
                .unwrap()
                .status,
            "blocked"
        );
        assert_eq!(cad_model::source_manifest(temp.path()).unwrap(), manifest);
        let source = cad_model::load_project(temp.path()).unwrap();
        assert!(matches!(
            source.drawings[0].entities[0].entity,
            cad_model::Entity::Line {
                p1: [100., 200.],
                p2: [1100., 2200.],
                ..
            }
        ));
    }

    #[test]
    fn unknown_suffix_invalid_scale_and_dropped_geometry_block() {
        let temp = tempfile::tempdir().unwrap();
        let mut data = symbol();
        data.push(0);
        assert_eq!(
            prepare_symbol_import(&data, "bad.jws", 100., temp.path())
                .unwrap()
                .status,
            "blocked"
        );
        assert_eq!(
            prepare_symbol_import(&symbol(), "bad.jws", 0., temp.path())
                .unwrap()
                .status,
            "blocked"
        );
        assert_eq!(
            prepare_symbol_import(&symbol(), "bad.jws", 0.0000001, temp.path())
                .unwrap()
                .status,
            "blocked"
        );
        assert!(!temp.path().join("cad.project.toml").exists());
    }

    fn class(writer: &mut cad_jww_codec::Writer, name: &str) {
        writer.u16(0xffff);
        writer.u16(600);
        writer.u16(name.len() as u16);
        writer.raw(name.as_bytes());
    }

    fn base(writer: &mut cad_jww_codec::Writer) {
        writer.u32(0);
        writer.u8(1);
        writer.u16(2);
        for _ in 0..4 {
            writer.u16(0);
        }
    }

    fn block(writer: &mut cad_jww_codec::Writer) {
        base(writer);
        for value in [5., 6., 1., 1., 0.] {
            writer.f64(value);
        }
        writer.u32(7);
    }

    fn nested_symbol(cycle: bool) -> Vec<u8> {
        let mut writer = cad_jww_codec::Writer::default();
        writer.raw(&symbol()[..452]);
        writer.u16(1);
        class(&mut writer, "CDataBlock");
        block(&mut writer);
        writer.u16(1);
        class(&mut writer, "CDataList");
        base(&mut writer);
        writer.u32(7);
        writer.u32(1);
        writer.u32(0);
        writer.cstring("part").unwrap();
        writer.u16(if cycle { 2 } else { 1 });
        class(&mut writer, "CDataSen");
        base(&mut writer);
        for value in [1., 2., 11., 22.] {
            writer.f64(value);
        }
        if cycle {
            writer.u16(0x8001);
            block(&mut writer);
        }
        writer.into_bytes()
    }

    #[test]
    fn block_placement_scales_once_and_recursion_blocks_partial_conversion() {
        let temp = tempfile::tempdir().unwrap();
        let report =
            prepare_symbol_import(&nested_symbol(false), "block.jws", 100., temp.path()).unwrap();
        assert_eq!(report.status, "converted");
        let source = cad_model::load_project(temp.path()).unwrap();
        assert!(matches!(
            source.drawings[0].entities[0].entity,
            cad_model::Entity::Line {
                p1: [600., 800.],
                p2: [1600., 2800.],
                ..
            }
        ));
        let second = tempfile::tempdir().unwrap();
        let blocked =
            prepare_symbol_import(&nested_symbol(true), "cycle.jws", 100., second.path()).unwrap();
        assert_eq!(blocked.status, "blocked");
        assert!(blocked.warnings.iter().any(|w| w.code == "block_cycle"));
        assert!(!second.path().join("cad.project.toml").exists());
        let third = tempfile::tempdir().unwrap();
        let scaled = prepare_symbol_import(&symbol(), "model.jws", 1., third.path()).unwrap();
        assert_eq!(scaled.status, "converted");
        let source = cad_model::load_project(third.path()).unwrap();
        let svg = cad_render_svg::render_project_svg(&source).unwrap();
        assert!(!svg.is_empty());
        let layout = &source.drawings[0].layouts.layouts["default"];
        assert_eq!(layout.origin, [-9., -8.]);
    }
}
