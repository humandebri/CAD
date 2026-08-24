# Plan 006: Make PDF output preserve canonical drawing styles

## Status

- **Implementation**: IN PROGRESS (implementation and macOS GUI gate complete; external review pending)
- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: Plans 001–005
- **Category**: output fidelity, macOS migration

## Why this matters

Jw_cad users judge migration quality by printed output. The previous PDF path
used one black stroke, Helvetica at a fixed size, replaced non-ASCII text with
`?`, and reduced dimensions and curve solids to placeholder geometry.

## Current state

- `cad-model` resolves layer defaults and entity pen overrides through one
  stroke-style API used by SVG and PDF.
- PDF emits canonical RGB, physical line width, scaled dash patterns, fill
  colors, dimension extension/arrow/text geometry, curve solids, and block
  transforms.
- Text height, width, spacing, alignment, rotation, mirroring, and Japanese
  Unicode text are preserved. The OFL-licensed M+ 1p font is deterministically
  subset and embedded as CIDFontType2 with a ToUnicode map.
- Unit tests validate style resolution, Unicode text, dimensions, xref object
  offsets, stale-source rejection, and atomic publication. A generated A4
  fixture is also parsed and rasterized with Poppler during local review.

## Boundaries and follow-up

- One bundled gothic/sans face currently covers all text families. Distinct
  mincho/serif fidelity requires a separately licensed face and remains a
  follow-up rather than relying on viewer substitution.
- The Linux Rust CI job parses, inspects embedded fonts and text extraction,
  and rasterizes the representative PDF with Poppler.
- JWW header color-table import and file-level fixture coverage remain in the
  JWW release gate.

## Done criteria

- [x] SVG and PDF resolve pen overrides and layer styles from one model API.
- [x] PDF renders color, line width, line type, fill, styled text, dimensions,
      block children, and curve solids.
- [x] Dimension lines, arrowheads, and text consistently use the resolved print
      color when display and print palettes differ.
- [x] Japanese text is retained as Unicode instead of replacement glyphs.
- [x] Japanese glyph subsets are embedded with deterministic CID and ToUnicode
      mappings.
- [x] PDF tests validate Type0/CID resources, content commands, and xref offsets.
- [x] Poppler accepts and rasterizes the representative Japanese fixture.
- [x] CI rejects missing font embedding, broken Japanese text extraction, or a
      PDF that Poppler cannot parse and rasterize.

## Verification

Run the complete workspace and desktop gates. For visual review, run the
Unicode/style PDF test with `CAD_PDF_TEST_OUTPUT` set, then inspect it using
`pdfinfo`, `pdffonts`, `pdftotext`, and `pdftoppm`.
