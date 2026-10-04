//! Editor schemas derived from the same serde types used by the source loader.
use crate::*;
use schemars::{JsonSchema, r#gen::SchemaGenerator, schema::Schema};
use serde_json::{Value, json};

impl JsonSchema for EntityId {
    fn schema_name() -> String {
        "EntityId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(json!({"type":"string", "pattern":"^ent_[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$", "description":"ent_ followed by a 26-character ULID; keep IDs when updating entities"})).expect("valid ID schema")
    }
}

pub fn schemas() -> BTreeMap<String, Value> {
    let mut schemas = BTreeMap::new();
    macro_rules! add {
        ($name:literal, $ty:ty) => {
            schemas.insert(
                $name.into(),
                serde_json::to_value(schemars::schema_for!($ty)).expect("schema serializes"),
            );
        };
    }
    add!("entity", Entity);
    add!("project", ProjectConfig);
    add!("layers", LayerRules);
    add!("styles", StyleRules);
    add!("layouts", LayoutsConfig);
    add!("block", BlockDefinitionConfig);
    for schema in schemas.values_mut() {
        constrain_version(schema);
    }
    schemas
}

fn constrain_version(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(version) = map
                .get_mut("properties")
                .and_then(|p| p.get_mut("schema_version"))
            {
                version["enum"] = json!([CURRENT_SCHEMA_VERSION]);
            }
            for child in map.values_mut() {
                constrain_version(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                constrain_version(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schemas_cover_every_variant_and_reject_extra_anchor_fields() {
        let schemas = schemas();
        let entity = &schemas["entity"];
        assert_eq!(entity["oneOf"].as_array().unwrap().len(), 12);
        assert!(
            entity["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["additionalProperties"] == false)
        );
        let anchors = &entity["definitions"]["DimensionAnchor"]["oneOf"];
        let fixed = anchors
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["kind"]["enum"] == json!(["fixed"]))
            .unwrap();
        assert!(fixed["properties"].get("point").is_some());
        assert!(fixed["properties"].get("at").is_none());
        assert_eq!(fixed["additionalProperties"], false);
        assert_eq!(
            schemas["project"]["properties"]["schema_version"]["enum"],
            json!([CURRENT_SCHEMA_VERSION])
        );
    }
}
