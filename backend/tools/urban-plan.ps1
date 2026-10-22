<#
.SYNOPSIS
  Draws the urban grid and measures it (#421), or Plots v2's (#455).

.DESCRIPTION
  Runs the ignored test urban::render::tests::urban_plan_report in release.
  It writes two SVG plan views, the whole ring from the outer canal to the
  suburb's edge and a close-up of the sector from 0 to 45 degrees, with
  urban.json and report.md (the plots plan's section 3 numbers) and
  index.html. Lots are shaded by area, red where one doesn't take 4
  stations, and edged orange where finish_lots trimmed one. Then it converts
  each SVG to a PNG with headless Chrome (or Edge).

.PARAMETER Seed
  The world seed. The grid has none, but the world round it does. Default: 42.

.PARAMETER Outer
  The urban edge's radius, to draw Plots v2's grid (#455): its test-only
  params (urban::metrics::v2), measured and drawn on the grid alone, since
  the land and the other zones don't follow the v2 radii yet. Without it,
  the default world. Plots v2 puts the edge at 2160.

.PARAMETER Rings
  Rings of blocks. One that won't fit inside the edge isn't laid. Default:
  the world's own (3 in the default world, 5 in v2's).

.PARAMETER OuterSpokes
  Bearings round the outer rings of blocks, past r1800 (urban.outer_spokes,
  #455); a multiple of 8, so the arterials are among them. 0 keeps the
  inner rings' 24. Default: the world's own.

.PARAMETER Out
  Output folder. Default: target/urban-plan at the repo root.

.PARAMETER Compare
  An earlier output folder. Writes compare.html, which shows each plan
  beside the one with the same name there and puts the two tables together.

.EXAMPLE
  powershell -File backend/tools/urban-plan.ps1
  powershell -File backend/tools/urban-plan.ps1 -Out target/urban-new -Compare target/urban-plan
  powershell -File backend/tools/urban-plan.ps1 -Outer 2160 -Out target/urban-v2
#>
param(
    [long]$Seed = 42,
    [double]$Outer = 0,
    [int]$Rings = 0,
    [int]$OuterSpokes = -1,
    [string]$Out = "",
    [string]$Compare = ""
)

$ErrorActionPreference = "Stop"
$backend = Split-Path $PSScriptRoot -Parent
$repo = Split-Path $backend -Parent
if (-not $Out) { $Out = Join-Path $repo "target/urban-plan" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

$env:URBAN_SEED = "$Seed"
$env:URBAN_OUT = $Out
$env:URBAN_OUTER = if ($Outer -gt 0) { "$Outer" } else { "" }
$env:URBAN_RINGS = if ($Rings -gt 0) { "$Rings" } else { "" }
$env:URBAN_OUTER_SPOKES = if ($OuterSpokes -ge 0) { "$OuterSpokes" } else { "" }

# Clear old plans out, so index.html and the PNGs match this run.
Get-ChildItem -Path (Join-Path $Out "*") -File -Include *.svg, *.png, *.json |
    ForEach-Object { Remove-Item $_.FullName -Force }

Push-Location $backend
try {
    cargo test --release --bin capitol-backend urban_plan_report -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw "urban_plan_report failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$browser = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1

if (-not $browser) {
    Write-Warning "No Chrome or Edge found: the SVGs are in $Out, but there are no PNGs."
} else {
    # A profile of its own, so a Chrome that's already open doesn't take the job.
    $userData = Join-Path $Out ".chrome-profile"
    foreach ($svg in Get-ChildItem $Out -Filter *.svg) {
        $head = Get-Content $svg.FullName -TotalCount 1
        if ($head -notmatch 'width="(\d+)" height="(\d+)"') { throw "$($svg.Name): no size on the <svg>" }
        $png = [System.IO.Path]::ChangeExtension($svg.FullName, ".png")
        $url = "file:///" + ($svg.FullName -replace '\\', '/')
        $chromeArgs = @(
            "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
            "--user-data-dir=`"$userData`"", "--window-size=$($Matches[1]),$($Matches[2])",
            "--screenshot=`"$png`"", "`"$url`""
        )
        Start-Process -FilePath $browser -ArgumentList $chromeArgs -Wait -WindowStyle Hidden
        if (Test-Path $png) { Write-Host "  $($svg.BaseName).png" } else { Write-Warning "$($svg.Name): no PNG" }
    }
    Remove-Item $userData -Recurse -Force -ErrorAction SilentlyContinue
}

if ($Compare) {
    if (-not [System.IO.Path]::IsPathRooted($Compare)) { $Compare = Join-Path (Get-Location) $Compare }
    $Compare = (Resolve-Path $Compare).Path
    $rel = [System.Uri]::new("$Out\").MakeRelativeUri([System.Uri]::new("$Compare\")).ToString()
    $rows = foreach ($svg in Get-ChildItem $Out -Filter *.svg) {
        $name = $svg.BaseName
        $old = if (Test-Path (Join-Path $Compare "$name.png")) { "$rel$name.png" } else { "$rel$name.svg" }
        $new = if (Test-Path (Join-Path $Out "$name.png")) { "$name.png" } else { "$name.svg" }
        "<h2>$name</h2><div class=`"pair`"><figure><img src=`"$old`"><figcaption>before: $Compare</figcaption></figure><figure><img src=`"$new`"><figcaption>now</figcaption></figure></div>"
    }
    $tables = foreach ($dir in @($Compare, $Out)) {
        $md = Join-Path $dir "report.md"
        if (Test-Path $md) { "<h2>$dir</h2><pre>" + [System.Net.WebUtility]::HtmlEncode((Get-Content $md -Raw -Encoding UTF8)) + "</pre>" }
    }
    $html = @"
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Urban plan compared</title>
<style>
body { margin: 0; padding: 24px 16px 48px; background: #f3eee2; color: #2e2a24; font: 15px/1.45 Georgia, serif; }
h2 { font-weight: normal; margin: 28px 0 8px; }
.pair { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 480px), 1fr)); gap: 16px; }
figure { margin: 0; } img { width: 100%; height: auto; border: 1px solid #d8cfbb; }
figcaption { color: #6b6253; margin-top: 4px; } pre { overflow-x: auto; font-size: 13px; }
</style></head><body>
$($rows -join "`n")
$($tables -join "`n")
</body></html>
"@
    Set-Content -Path (Join-Path $Out "compare.html") -Value $html -Encoding UTF8
    Write-Host "compare: $(Join-Path $Out 'compare.html')"
}

Write-Host "plan: $(Join-Path $Out 'index.html')"
