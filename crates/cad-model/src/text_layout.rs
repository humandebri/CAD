//! Explicit upright annotation columns, independent of font vertical substitutions.
use crate::{BBox, Point, TextAlign, TextStyleDef};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextWritingMode {
    #[default]
    Horizontal,
    VerticalUpright,
}
impl TextWritingMode {
    pub fn is_horizontal(&self) -> bool {
        *self == Self::Horizontal
    }
}

/// Each Unicode scalar occupies one upright cell. LF starts a column to the left.
/// `at` is the first column's top-left anchor before rotation and mirroring.
pub fn upright_text_glyphs(
    at: Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
) -> Vec<(Point, char)> {
    let angle = rotation_deg.to_radians();
    let (sin, cos) = angle.sin_cos();
    let sign = if mirror_y { -1. } else { 1. };
    let mut glyphs = Vec::new();
    for (column, text) in value.split('\n').enumerate() {
        let count = text.chars().count();
        let length = count as f64 * style.height + count.saturating_sub(1) as f64 * style.spacing;
        let offset = match style.align {
            TextAlign::Left => 0.,
            TextAlign::Center => length / 2.,
            TextAlign::Right => length,
        };
        let x = -(column as f64) * (style.width + style.spacing);
        for (row, character) in text.chars().enumerate() {
            let y = offset - style.height - row as f64 * (style.height + style.spacing);
            glyphs.push((
                [
                    at[0] + cos * x - sin * sign * y,
                    at[1] + sin * x + cos * sign * y,
                ],
                character,
            ));
        }
    }
    glyphs
}

pub fn upright_text_bbox(
    at: Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
) -> Option<BBox> {
    let angle = rotation_deg.to_radians();
    let (sin, cos) = angle.sin_cos();
    let sign = if mirror_y { -1. } else { 1. };
    let points = upright_text_glyphs(at, rotation_deg, mirror_y, value, style)
        .into_iter()
        .flat_map(|(p, _)| {
            [
                [0., 0.],
                [style.width, 0.],
                [0., style.height],
                [style.width, style.height],
            ]
            .map(|[x, y]| {
                [
                    p[0] + cos * x - sin * sign * y,
                    p[1] + sin * x + cos * sign * y,
                ]
            })
        })
        .collect::<Vec<_>>();
    if points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    if points.is_empty() {
        BBox::from_points(&[at])
    } else {
        BBox::from_points(&points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upright_columns_use_height_spacing_alignment_rotation_and_mirroring() {
        let mut style = TextStyleDef {
            font_family: "test".into(),
            height: 10.,
            width: 5.,
            spacing: 2.,
            align: TextAlign::Left,
        };
        let positions = upright_text_glyphs([100., 200.], 0., false, "室名\nA2", &style);
        assert_eq!(
            positions,
            vec![
                ([100., 190.], '室'),
                ([100., 178.], '名'),
                ([93., 190.], 'A'),
                ([93., 178.], '2')
            ]
        );
        assert_eq!(
            upright_text_bbox([100., 200.], 0., false, "室名\nA2", &style),
            Some(BBox {
                min: [93., 178.],
                max: [105., 200.]
            })
        );
        style.align = TextAlign::Center;
        assert_eq!(
            upright_text_glyphs([0., 0.], 0., false, "AB", &style)[0].0,
            [0., 1.]
        );
        style.align = TextAlign::Right;
        assert_eq!(
            upright_text_glyphs([0., 0.], 0., false, "AB", &style)[1].0,
            [0., 0.]
        );
        style.align = TextAlign::Left;
        let rotated = upright_text_glyphs([0., 0.], 90., true, "A", &style)[0].0;
        assert!((rotated[0] + 10.).abs() < 1e-10 && rotated[1].abs() < 1e-10);
    }
    #[test]
    fn legacy_annotations_keep_horizontal_serialization() {
        let raw = r#"{"schema_version":"0.3","type":"text","id":"ent_01ARZ3NDEKTSV4RRFFQ69G5FAV","layer":"0-1","style":"note","at":[0,0],"rotation_deg":0,"value":"室名"}"#;
        let entity: crate::Entity = serde_json::from_str(raw).unwrap();
        assert!(
            !serde_json::to_value(entity)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("writing_mode")
        );
    }
}
