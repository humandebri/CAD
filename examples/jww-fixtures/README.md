# JWW Fixtures

These fixtures cover JWW inspection, byte-exact original retention, import,
preservation export, generated export, and re-import regression tests.
`manifest.json` fixes the required hashes and decoded record inventory.
External application checks are optional evidence, not a release gate.

## Test1.jww

- Source repository: <https://github.com/monozukuri-ai/ezjww>
- Source path: `jww_samples/Test1.jww`
- License: MIT License
- Copyright notice in source repository: `Copyright (c) 2026 k-tanaka`
- Retrieved: 2026-07-09

The MIT license text is recorded in this file so the fixture can be redistributed
with this repository.

```text
MIT License

Copyright (c) 2026 k-tanaka

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Additional upstream fixtures (2026-09-22)

The following files use the same upstream MIT license reproduced above, pinned to
commit `4547d5d3d0226505ccb8626de8b21f09037cd088` of `monozukuri-ai/ezjww`.
Their exact SHA-256 and inventories are recorded in `manifest.json`.

- `apartment-plan.jww`: `jww_samples/Ａマンション平面例.jww`, v600,
  including 79 `CDataSolid` records. Crossing corner orders are normalized only
  in the canonical polygon with `solid_boundary_reordered` import warnings;
  raw provenance remains untouched. Tests preserve every decoded solid field
  and the entire decoded header/palette while editing a line and re-importing.
- `2blocks.jww`: `jww_samples/block_regressions/2blocks.jww`, v700.
  This is **unsupported-version preservation coverage only**. Its records are
  not parsed or claimed compatible with the v600 editor; original extraction
  is checked byte-for-byte.

All manifest entries are hash/inventory checked, not just the first entry.
Licensed native v600 dimension, block, and hatch fixtures are still missing.
Synthetic tests cover these paths but do not establish Jw_cad interoperability.
