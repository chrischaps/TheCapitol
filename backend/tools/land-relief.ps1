<#
.SYNOPSIS
  Draws the land's height as a shaded relief, for some seeds (W2, #195).

.DESCRIPTION
  Runs the ignored test land::tests::land_relief in capitol-protocol, in
  release. For each seed it writes relief-<seed>.png: the whole world disc,
  tinted by height (meadow, forest, rock, snow) under a hillshade lit from
  the north-west, with the city's zone rings faint in the middle. It prints
  the wild heights' median, p90, p99, p99.9 and max beside each one.
  The sibling of suburb-plan.ps1; the PNGs need no browser.

.PARAMETER Seeds
  Comma-separated land seeds. Default: 42,7,1234.

.PARAMETER Size
  The PNG's width and height in pixels. Default: 1200 (10 units a pixel).

.PARAMETER Out
  Output folder. Default: target/land-relief at the repo root.

.EXAMPLE
  powershell -File backend/tools/land-relief.ps1
  powershell -File backend/tools/land-relief.ps1 -Seeds 42 -Size 2400
#>
param(
    [string]$Seeds = "42,7,1234",
    [int]$Size = 1200,
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$backend = Split-Path $PSScriptRoot -Parent
$repo = Split-Path $backend -Parent
if (-not $Out) { $Out = Join-Path $repo "target/land-relief" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

Push-Location (Join-Path $repo "protocol")
try {
    foreach ($seed in $Seeds -split ",") {
        $seed = $seed.Trim()
        $env:LAND_SEED = $seed
        $env:LAND_SIZE = "$Size"
        $env:LAND_RELIEF = Join-Path $Out "relief-$seed.png"
        cargo test --release -p capitol-protocol --lib land_relief -- --ignored --nocapture --quiet |
            Select-String "^relief:"
        if ($LASTEXITCODE -ne 0) { throw "land_relief failed for seed $seed ($LASTEXITCODE)" }
    }
} finally {
    Pop-Location
}
