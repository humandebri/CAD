# Plan 007: Preserve JWW display and print palettes

## Status

- **Implementation**: IN PROGRESS (implementation and macOS GUI gate complete; external review pending)
- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: Plans 004 and 006
- **Category**: JWW compatibility, display and print fidelity

## Current state

- The v600 codec decodes and encodes all ten basic screen colors, screen
  widths, print colors, print widths, and print point radii.
- JWW `COLORREF` values are converted to canonical `#RRGGBB` without swapping
  red and blue.
- Imported styles retain every palette entry, including currently unused
  colors, so later editing and export do not restore application defaults.
- `ColorDef.rgb` is the display color and optional `print_rgb` is the print
  color. SVG uses the display color; PDF uses the print color.
- An entity pen width of zero resolves through the corresponding JWW print
  width table entry during import, so PDF does not fall back to a fixed width.
- Export reconstructs both JWW color tables and the print-width table. The
  public `Test1.jww` fixture proves table equality across import/export.

## Done criteria

- [x] Decode and encode v600 display and print palette values.
- [x] Preserve all ten basic entries through import/export.
- [x] Preserve distinct display and print colors in the canonical style model.
- [x] Use print colors for PDF and screen colors for SVG.
- [x] Preserve arbitrary solid colors using JWW `COLORREF` byte order.
- [x] Validate malformed `print_rgb` values in the checker.
- [ ] Pass the external read-only review gate.
- [x] Pass the macOS Desktop GUI gate outside the restricted sandbox.
- [ ] Add licensed palette fixtures to the hashed file-level manifest before
      expanding the claimed palette coverage.
