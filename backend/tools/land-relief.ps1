<#
.SYNOPSIS
  Draws the land's height as a shaded relief, with the wilds' water over it,
  for some seeds (W2 #195, W3 #196).

.DESCRIPTION
  Runs the ignored test models::hydrology::tests::water_plan in the backend,
  in release. For each seed it writes relief-<seed>.png: the whole world disc,
  tinted by height (meadow, forest, rock, snow) under a hillshade lit from
  the north-west, with the city's zone rings faint in the middle. The ground
  is carved (land::carve), and the lakes, rivers (dark), streams (light) and
  fords (orange dots) are drawn over it, and the wild trails (W5, red) over
  that, so their bridges show. It prints the wild heights' median,
  p90, p99 and max and the water's counts beside each one.
  With -Nodes it draws the planned nodes past the suburb over it all (W6):
  trees dark green, branches brown, stone white, clay orchid, iron veins gold.
  With -Bare it draws the land alone, from capitol-protocol's land_relief,
  and adds p99.9.
  The sibling of suburb-plan.ps1; the PNGs need no browser.

.PARAMETER Seeds
  Comma-separated land seeds. Default: 42,7,1234.

.PARAMETER Size
  The PNG's width and height in pixels. Default: 1200 (10 units a pixel).

.PARAMETER Out
  Output folder. Default: target/land-relief at the repo root.

.PARAMETER Bare
  The land alone, with no water.

.PARAMETER Nodes
  The planned resource nodes too (best at -Size 2400 or more).

.EXAMPLE
  powershell -File backend/tools/land-relief.ps1
  powershell -File backend/tools/land-relief.ps1 -Seeds 42 -Size 2400
  powershell -File backend/tools/land-relief.ps1 -Bare
  powershell -File backend/tools/land-relief.ps1 -Seeds 42 -Size 3000 -Nodes
#>
param(
    [string]$Seeds = "42,7,1234",
    [int]$Size = 1200,
    [string]$Out = "",
    [switch]$Bare,
    [switch]$Nodes
)

$ErrorActionPreference = "Stop"
$backend = Split-Path $PSScriptRoot -Parent
$repo = Split-Path $backend -Parent
if (-not $Out) { $Out = Join-Path $repo "target/land-relief" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

Push-Location (Join-Path $repo $(if ($Bare) { "protocol" } else { "backend" }))
try {
    foreach ($seed in $Seeds -split ",") {
        $seed = $seed.Trim()
        $png = Join-Path $Out "relief-$seed.png"
        $env:LAND_SEED = $seed
        $env:LAND_SIZE = "$Size"
        $env:LAND_RELIEF = $png
        $env:WATER_PLAN = $png
        if ($Nodes) { $env:WILD_NODES = "1" } else { Remove-Item Env:WILD_NODES -ErrorAction SilentlyContinue }
        if ($Bare) {
            cargo test --release -p capitol-protocol --lib land_relief -- --ignored --nocapture --quiet |
                Select-String "^relief:"
        } else {
            cargo test --release --bin capitol-backend water_plan -- --ignored --nocapture --quiet |
                Select-String "^relief:"
        }
        if ($LASTEXITCODE -ne 0) { throw "the relief failed for seed $seed ($LASTEXITCODE)" }
    }
} finally {
    Pop-Location
}
