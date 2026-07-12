# JWW Compatibility Checklist

Use `validation/record-template.md` for each Windows/Jw_cad run. Store the
completed record under `validation/<yyyy-mm-dd>-<fixture>/` together with
screenshots and the generated JSON reports.

The automated Rust tests cover codec parsing/writing, the public `Test1.jww`
fixture, synthetic solids, strict blockers, lossy reports, and
import/export/re-import validation. The following checks require Windows Jw_cad
and must be recorded before removing the Experimental label.

- [ ] Record the Jw_cad version, Windows version, fixture name, and SHA-256 values.
- [ ] Open a JWW produced from `Test1.jww` import/export without a repair prompt.
- [ ] Verify line, arc, circle, ellipse, text, dimension, and point geometry.
- [ ] Verify layer group names, layer order, visibility, lock state, and active layer.
- [ ] Verify polygon solid, circle solid, ellipse solid, arc solid, and ring solid display using a licensed fixture.
- [ ] Verify CP932 text and non-ASCII project names display correctly.
- [ ] Save the file from Jw_cad, re-import it, and confirm the CAD checker passes.
- [ ] Attach the fixture name, checksum, screenshots, and observed warnings to the validation record.

For every failure, record the smallest reproducible fixture, entity type,
layer, coordinates, JWW record/class, and the failing stage (codec, import,
export, Jw_cad open, or re-import). Convert confirmed regressions into Rust
tests before changing the Experimental gate.

Until every item is checked, `Export JWW (Experimental)` and the CLI
Experimental description must remain unchanged.
