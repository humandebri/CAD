# JWW Validation Records

This directory stores optional external-application compatibility evidence.
The required gate is the machine-readable fixture manifest plus automated
codec, byte-exact export, and semantic re-import checks.

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

External records should identify the exact application and operating-system
versions used, but they do not control release status.

The legacy two-stage PowerShell workflow remains available for optional Jw_cad
evidence:

```powershell
./scripts/windows-jww-gate.ps1 -Mode prepare -Fixture ./examples/jww-fixtures/Test1.jww -ArtifactDir ./examples/jww-fixtures/validation/2026-08-23-test1
# Open generated-preserved.jww in Jw_cad and Save As jwcad-saved.jww.
./scripts/windows-jww-gate.ps1 -Mode finalize -Fixture ./examples/jww-fixtures/Test1.jww -ArtifactDir ./examples/jww-fixtures/validation/2026-08-23-test1
```

The script records hashes, codec inspection, import, preserved export, saved-file
re-import, and checker status. It is supplementary to the required file-level
CI checks.
