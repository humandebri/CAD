# JWW Validation Records

This directory stores results from the Windows/Jw_cad compatibility gate.
The Mac-based CI verifies codec parsing/writing and import/export/re-import;
these records provide the separate application-level confirmation.

Create one directory per run:

```text
validation/2026-07-12-test1/
  record.md
  input.sha256
  exported.sha256
  export-report.json
  checker-report.json
  screenshots/
```

Do not commit licensed source fixtures or generated JWW files unless their
redistribution terms are recorded. Store checksums and a path/reference to the
licensed fixture in `record.md` instead.

The Experimental label may be removed only after every checklist item is
completed for the required fixture set and the records identify the exact
Windows and Jw_cad versions used.
