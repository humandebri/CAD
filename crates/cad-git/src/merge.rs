//! Entity/field-aware three-way merge. Conflicts retain ours in the candidate;
//! callers must not publish a conflicted or checker-invalid candidate.
use crate::{GitError, Result, Snapshot, SnapshotIdentity, source_files};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct MergeConflict {
    pub file: String,
    pub field: String,
    pub kind: String,
    pub base: Option<Value>,
    pub ours: Option<Value>,
    pub theirs: Option<Value>,
}
#[derive(Debug, Serialize)]
pub struct MergeReport {
    pub schema_version: String,
    pub base: SnapshotIdentity,
    pub ours: SnapshotIdentity,
    pub theirs: SnapshotIdentity,
    pub conflicts: Vec<MergeConflict>,
}
pub struct MergeCandidate {
    pub files: BTreeMap<String, Vec<u8>>,
    pub report: MergeReport,
}

pub fn merge(base: &Snapshot, ours: &Snapshot, theirs: &Snapshot) -> Result<MergeCandidate> {
    fn files(snapshot: &Snapshot) -> Result<BTreeMap<String, Vec<u8>>> {
        source_files(&snapshot.source.root)?
            .into_iter()
            .map(|(name, bytes)| {
                Ok((
                    name.to_str()
                        .ok_or_else(|| GitError::Invalid("non-UTF8 source path".into()))?
                        .to_owned(),
                    bytes,
                ))
            })
            .collect()
    }
    let base_files = files(base)?;
    let ours_files = files(ours)?;
    let theirs_files = files(theirs)?;
    let names: BTreeSet<_> = base_files
        .keys()
        .chain(ours_files.keys())
        .chain(theirs_files.keys())
        .cloned()
        .collect();
    let mut conflicts = Vec::new();
    let mut output = BTreeMap::new();
    for file in names {
        let b = base_files.get(&file);
        let o = ours_files.get(&file);
        let t = theirs_files.get(&file);
        let bytes = if o == t || t == b {
            o.cloned()
        } else if o == b {
            t.cloned()
        } else if let (Some(o), Some(t)) = (o, t) {
            if file.ends_with("/entities.ndjson") {
                Some(merge_entities(
                    &file,
                    b.map(Vec::as_slice),
                    o,
                    t,
                    &mut conflicts,
                )?)
            } else if file.ends_with(".toml") && !file.starts_with("interop/") {
                let decode = |bytes: &[u8]| -> Result<Value> {
                    let text =
                        std::str::from_utf8(bytes).map_err(|e| GitError::Invalid(e.to_string()))?;
                    let value: toml::Value =
                        toml::from_str(text).map_err(|e| GitError::Invalid(e.to_string()))?;
                    serde_json::to_value(value).map_err(|e| GitError::Invalid(e.to_string()))
                };
                let base = b.map(|bytes| decode(bytes)).transpose()?;
                let ours = decode(o)?;
                let theirs = decode(t)?;
                let result = merge_value(
                    &file,
                    "",
                    base.as_ref(),
                    Some(&ours),
                    Some(&theirs),
                    &mut conflicts,
                )
                .expect("both objects present");
                let value: toml::Value =
                    serde_json::from_value(result).map_err(|e| GitError::Invalid(e.to_string()))?;
                Some(
                    toml::to_string_pretty(&value)
                        .map_err(|e| GitError::Invalid(e.to_string()))?
                        .into_bytes(),
                )
            } else {
                file_conflict(&file, b, Some(o), Some(t), &mut conflicts);
                Some(o.clone())
            }
        } else {
            file_conflict(&file, b, o, t, &mut conflicts);
            o.cloned()
        };
        if let Some(bytes) = bytes {
            output.insert(file, bytes);
        }
    }
    Ok(MergeCandidate {
        files: output,
        report: MergeReport {
            schema_version: "cad-merge/1".into(),
            base: base.identity.clone(),
            ours: ours.identity.clone(),
            theirs: theirs.identity.clone(),
            conflicts,
        },
    })
}

fn file_conflict(
    file: &str,
    b: Option<&Vec<u8>>,
    o: Option<&Vec<u8>>,
    t: Option<&Vec<u8>>,
    conflicts: &mut Vec<MergeConflict>,
) {
    let digest = |bytes: Option<&Vec<u8>>| {
        bytes.map(|bytes| Value::String(blake3::hash(bytes).to_hex().to_string()))
    };
    conflicts.push(MergeConflict {
        file: file.into(),
        field: "/".into(),
        kind: if o.is_none() || t.is_none() {
            "delete_modify"
        } else {
            "file_changed"
        }
        .into(),
        base: digest(b),
        ours: digest(o),
        theirs: digest(t),
    });
}

fn merge_value(
    file: &str,
    path: &str,
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    conflicts: &mut Vec<MergeConflict>,
) -> Option<Value> {
    if ours == theirs || theirs == base {
        return ours.cloned();
    }
    if ours == base {
        return theirs.cloned();
    }
    if let (Some(Value::Object(o)), Some(Value::Object(t))) = (ours, theirs)
        && base.is_none_or(Value::is_object)
    {
        let b = base.and_then(Value::as_object);
        let keys: BTreeSet<_> = o
            .keys()
            .chain(t.keys())
            .chain(b.into_iter().flat_map(|v| v.keys()))
            .collect();
        let mut output = serde_json::Map::new();
        for key in keys {
            let escaped = key.replace('~', "~0").replace('/', "~1");
            let path = format!("{path}/{escaped}");
            if let Some(value) = merge_value(
                file,
                &path,
                b.and_then(|b| b.get(key)),
                o.get(key),
                t.get(key),
                conflicts,
            ) {
                output.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(output));
    }
    conflicts.push(MergeConflict {
        file: file.into(),
        field: path.into(),
        kind: if ours.is_none() || theirs.is_none() {
            "delete_modify"
        } else {
            "field_changed"
        }
        .into(),
        base: base.cloned(),
        ours: ours.cloned(),
        theirs: theirs.cloned(),
    });
    ours.cloned()
}

struct Records {
    order: Vec<String>,
    values: BTreeMap<String, Value>,
    raw: BTreeMap<String, String>,
}
fn records(bytes: &[u8]) -> Result<Records> {
    let text = std::str::from_utf8(bytes).map_err(|e| GitError::Invalid(e.to_string()))?;
    let mut output = Records {
        order: Vec::new(),
        values: BTreeMap::new(),
        raw: BTreeMap::new(),
    };
    for line in text.lines() {
        let entity: Value =
            serde_json::from_str(line).map_err(|e| GitError::Invalid(e.to_string()))?;
        let id = entity["id"]
            .as_str()
            .ok_or_else(|| GitError::Invalid("merge entity has no ID".into()))?
            .to_owned();
        if output.values.insert(id.clone(), entity).is_some() {
            return Err(GitError::Invalid(format!("duplicate merge entity {id}")));
        }
        output.raw.insert(id.clone(), line.to_owned());
        output.order.push(id);
    }
    Ok(output)
}
fn merge_entities(
    file: &str,
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    conflicts: &mut Vec<MergeConflict>,
) -> Result<Vec<u8>> {
    let b = records(base.unwrap_or_default())?;
    let o = records(ours)?;
    let t = records(theirs)?;
    let newline = if ours.windows(2).any(|bytes| bytes == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut visited = BTreeSet::new();
    let mut lines = Vec::new();
    for id in o.order.iter().chain(&t.order).chain(&b.order) {
        if !visited.insert(id) {
            continue;
        }
        if let Some(value) = merge_value(
            file,
            &format!("/{id}"),
            b.values.get(id),
            o.values.get(id),
            t.values.get(id),
            conflicts,
        ) {
            let original = if o.values.get(id) == Some(&value) {
                o.raw.get(id)
            } else if t.values.get(id) == Some(&value) {
                t.raw.get(id)
            } else {
                None
            };
            lines.push(match original {
                Some(line) => line.clone(),
                None => {
                    serde_json::to_string(&value).map_err(|e| GitError::Invalid(e.to_string()))?
                }
            });
        }
    }
    let mut text = lines.join(newline);
    if !text.is_empty() && (ours.ends_with(b"\n") || ours.is_empty()) {
        text.push_str(newline);
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn disjoint_fields_combine_but_coordinates_and_deletion_conflict() {
        let base = json!({"id":"same","p1":[0,0],"p2":[10,0],"layer":"wall"});
        let ours = json!({"id":"same","p1":[0,0],"p2":[20,0],"layer":"wall"});
        let theirs = json!({"id":"same","p1":[0,0],"p2":[10,0],"layer":"detail"});
        let mut conflicts = Vec::new();
        let combined = merge_value(
            "entities.ndjson",
            "/same",
            Some(&base),
            Some(&ours),
            Some(&theirs),
            &mut conflicts,
        )
        .unwrap();
        assert_eq!(combined["p2"], json!([20, 0]));
        assert_eq!(combined["layer"], "detail");
        assert!(conflicts.is_empty());
        let other = json!({"id":"same","p1":[0,0],"p2":[30,0],"layer":"wall"});
        merge_value(
            "entities.ndjson",
            "/same",
            Some(&base),
            Some(&ours),
            Some(&other),
            &mut conflicts,
        );
        assert_eq!(conflicts[0].field, "/same/p2");
        conflicts.clear();
        assert!(
            merge_value(
                "entities.ndjson",
                "/same",
                Some(&base),
                None,
                Some(&other),
                &mut conflicts
            )
            .is_none()
        );
        assert_eq!(conflicts[0].kind, "delete_modify");
    }
    #[test]
    fn entity_merge_keeps_unmodified_bytes_order_and_line_endings() {
        let base = b"{\"id\":\"a\", \"x\":1}\r\n{\"id\":\"b\",\"x\":1}\r\n";
        let ours = b"{\"id\":\"a\", \"x\":2}\r\n{\"id\":\"b\",\"x\":1}\r\n";
        let theirs =
            b"{\"id\":\"a\", \"x\":1}\r\n{\"id\":\"b\",\"x\":3}\r\n{\"id\":\"c\",\"x\":4}\r\n";
        let mut conflicts = Vec::new();
        let output =
            merge_entities("entities.ndjson", Some(base), ours, theirs, &mut conflicts).unwrap();
        assert!(conflicts.is_empty());
        assert_eq!(
            output,
            b"{\"id\":\"a\", \"x\":2}\r\n{\"id\":\"b\",\"x\":3}\r\n{\"id\":\"c\",\"x\":4}\r\n"
        );
        assert!(records(b"{\"id\":\"a\"}\n{\"id\":\"a\"}\n").is_err());
    }
}
