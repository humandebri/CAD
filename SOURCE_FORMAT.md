# Canonical CAD source editing

JWW is imported into this repository's schema `0.2` project format. The TOML
and NDJSON files are then edited directly; `interop/jww/` is preservation data,
not editable source.

## Edit and validate

Each non-empty `entities.ndjson` line is one JSON object. Keep an existing
`id` when changing an entity and create a new valid, unique ID only for a new
entity. Layers, pens, text styles, dimension styles, fills, and blocks must
reference definitions in `rules/` or `blocks/`.

```bash
cargo run -p cad-cli -- check <PROJECT> --target cad --format json --out -
cargo run -p cad-cli -- check <PROJECT> --drawing <NAME> --target jww-v600 --format json --out -
cargo run -p cad-cli -- export-jww <PROJECT> --drawing <NAME> --out output.jww --report output.json
```

Normal JWW export is best-effort. Use `--strict` to reject every reported
approximation or substitution.

## Entity examples

These examples use the same layer and style identifiers as
`examples/house-small`. Each object must occupy one physical NDJSON line.

```json
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0,0],"p2":[1000,0]}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0,0],[1000,0],[1000,500]],"closed":false}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"arc","layer":"0-1","center":[0,0],"radius":500,"start_deg":0,"end_deg":90}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"circle","layer":"0-1","center":[0,0],"radius":500}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"ellipse","layer":"0-1","center":[0,0],"radius_x":500,"radius_y":250,"rotation_deg":0,"start_deg":0,"end_deg":360}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000005","type":"text","layer":"0-1","style":"note","at":[0,0],"rotation_deg":0,"mirror_y":false,"value":"室名"}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000006","type":"dimension","layer":"0-1","style":"dim_100","p1":[0,0],"p2":[1000,0],"offset":200,"text_rotation_deg":0,"text_mirror_y":false,"value":null}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000007","type":"point","layer":"0-1","at":[0,0],"temporary":false,"marker_code":null,"rotation_deg":0,"scale":1}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000008","type":"solid","layer":"0-1","points":[[0,0],[100,0],[0,100]],"fill":"jw_black"}
{"schema_version":"0.2","id":"ent_01JZ0000000000000000000009","type":"curve_solid","layer":"0-1","center":[0,0],"radius":500,"flatness":1,"rotation_deg":0,"start_deg":0,"end_deg":180,"solid_param":100,"encoding_code":101,"fill":"jw_black"}
{"schema_version":"0.2","id":"ent_01JZ000000000000000000000A","type":"block_ref","layer":"0-1","block":"door","at":[0,0],"rotation_deg":0,"scale":1}
{"schema_version":"0.2","id":"ent_01JZ000000000000000000000B","type":"hatch","layer":"0-1","loops":[[[0,0],[1000,0],[1000,1000],[0,1000]]],"pattern":"solid","angle_deg":0,"scale":1,"fill":"jw_black"}
```

Move, copy, rotate, mirror, scale, stretch, trim, extend, split, join,
chamfer, and fillet results are represented by updating these coordinates or
by replacing an entity with the resulting line and arc primitives. They do not
require Jw_cad command objects in the source schema.
