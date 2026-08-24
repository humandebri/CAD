[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("prepare", "finalize")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$Fixture,

    [Parameter(Mandatory = $true)]
    [string]$ArtifactDir
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$FixturePath = [System.IO.Path]::GetFullPath($Fixture)
$ArtifactPath = [System.IO.Path]::GetFullPath($ArtifactDir)
$ImportedProject = Join-Path $ArtifactPath "imported"
$GeneratedJww = Join-Path $ArtifactPath "generated-preserved.jww"
$SavedJww = Join-Path $ArtifactPath "jwcad-saved.jww"
$ReimportedProject = Join-Path $ArtifactPath "reimported"

function Write-Sha256([string]$Path, [string]$Destination) {
    $Hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    Set-Content -LiteralPath $Destination -Value "$Hash  $([System.IO.Path]::GetFileName($Path))" -Encoding utf8
}

Push-Location $RepoRoot
try {
    if ($Mode -eq "prepare") {
        if (Test-Path -LiteralPath $ArtifactPath) {
            throw "Artifact directory already exists; choose a new run directory: $ArtifactPath"
        }
        New-Item -ItemType Directory -Path $ArtifactPath | Out-Null
        Copy-Item -LiteralPath "examples/jww-fixtures/validation/record-template.md" -Destination (Join-Path $ArtifactPath "record.md")

        & cargo run -p cad-cli -- inspect-jww $FixturePath |
            Set-Content -LiteralPath (Join-Path $ArtifactPath "inspection.json") -Encoding utf8
        if ($LASTEXITCODE -ne 0) { throw "inspect-jww failed" }
        & cargo run -p cad-cli -- import-jww $FixturePath --out $ImportedProject
        if ($LASTEXITCODE -ne 0) { throw "import-jww failed" }
        $DrawingName = [System.IO.Path]::GetFileNameWithoutExtension($FixturePath).ToLowerInvariant()
        & cargo run -p cad-cli -- export-jww $ImportedProject --drawing $DrawingName --out $GeneratedJww --preserve --report (Join-Path $ArtifactPath "export-report.json")
        if ($LASTEXITCODE -ne 0) { throw "preserved export failed" }

        Write-Sha256 $FixturePath (Join-Path $ArtifactPath "input.sha256")
        Write-Sha256 $GeneratedJww (Join-Path $ArtifactPath "generated.sha256")
        Write-Host "Open $GeneratedJww in Jw_cad, inspect it, then Save As $SavedJww"
        Write-Host "After saving, run this script again with -Mode finalize and the same arguments."
    }
    else {
        if (-not (Test-Path -LiteralPath $SavedJww -PathType Leaf)) {
            throw "Jw_cad-saved file is missing: $SavedJww"
        }
        if (Test-Path -LiteralPath $ReimportedProject) {
            throw "Re-import destination already exists: $ReimportedProject"
        }
        Write-Sha256 $SavedJww (Join-Path $ArtifactPath "jwcad-saved.sha256")
        & cargo run -p cad-cli -- inspect-jww $SavedJww |
            Set-Content -LiteralPath (Join-Path $ArtifactPath "jwcad-saved-inspection.json") -Encoding utf8
        if ($LASTEXITCODE -ne 0) { throw "saved-file inspection failed" }
        & cargo run -p cad-cli -- import-jww $SavedJww --out $ReimportedProject
        if ($LASTEXITCODE -ne 0) { throw "Jw_cad-saved file re-import failed" }
        & cargo run -p cad-cli -- check $ReimportedProject --format json --out (Join-Path $ArtifactPath "checker-report.json")
        if ($LASTEXITCODE -ne 0) { throw "re-imported project checker failed" }
        Write-Host "Automated finalize checks passed. Complete record.md and attach screenshots before accepting the gate."
    }
}
finally {
    Pop-Location
}
