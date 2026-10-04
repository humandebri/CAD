#!/usr/bin/env python3
"""Validate each canonical example record/config against model-generated schemas."""
import argparse
import copy
import json
from pathlib import Path
import tomllib
from jsonschema import Draft7Validator

parser = argparse.ArgumentParser()
parser.add_argument("schemas", type=Path)
parser.add_argument("project", type=Path)
args = parser.parse_args()
validators = {}
for name in ("entity", "project", "layers", "styles", "layouts", "block", "draft-request"):
    schema = json.loads((args.schemas / f"{name}.schema.json").read_text())
    Draft7Validator.check_schema(schema)
    validators[name] = Draft7Validator(schema)

count = 0

def validate(name, value, location):
    global count
    errors = sorted(validators[name].iter_errors(value), key=lambda e: str(e.path))
    if errors:
        raise SystemExit(f"{location}: {errors[0].message}")
    count += 1

configs = [("project", args.project / "cad.project.toml"),
           ("layers", args.project / "rules/layers.toml"),
           ("styles", args.project / "rules/styles.toml")]
configs += [("layouts", p) for p in sorted(args.project.glob("drawings/*/layouts.toml"))]
configs += [("block", p) for p in sorted(args.project.glob("blocks/*/definition.toml"))]
for name, path in configs:
    validate(name, tomllib.loads(path.read_text()), path)

kinds = set()
fixed_dimension = None
for path in sorted(args.project.glob("drawings/*/entities.ndjson")) + sorted(args.project.glob("blocks/*/entities.ndjson")):
    for line, record in enumerate(path.read_text().splitlines(), 1):
        entity = json.loads(record)
        validate("entity", entity, f"{path}:{line}")
        kinds.add(entity["type"])
        if entity.get("measurement", {}).get("first", {}).get("kind") == "fixed":
            fixed_dimension = entity
expected = {"line", "polyline", "arc", "circle", "ellipse", "text", "dimension", "point", "solid", "curve_solid", "block_ref", "hatch"}
assert kinds == expected, (kinds, expected)
assert fixed_dimension is not None
bad = copy.deepcopy(fixed_dimension)
anchor = bad["measurement"]["first"]
anchor["at"] = anchor.pop("point")
assert not validators["entity"].is_valid(bad), "at must not pass as a fixed anchor"
for field, value in [("schema_version", "0.2"), ("id", "ent_80000000000000000000000000"), ("unknown_field", True)]:
    bad = {**fixed_dimension, field: value}
    assert not validators["entity"].is_valid(bad), f"invalid {field} passed"
root = Path(__file__).resolve().parent.parent
for path in sorted((root / "docs/direct-edit").glob("*.json")):
    validate("draft-request", json.loads(path.read_text()), path)
print(f"Validated {count} examples/configurations; all 12 entity kinds and negative anchor/schema/ID cases passed")
