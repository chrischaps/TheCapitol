<#
.SYNOPSIS
  Draws the default world's resource nodes by type (E2.3, #162).

.DESCRIPTION
  Runs the ignored test models::resource::tests::node_plan_svg in release. It
  writes three SVGs: the whole world, the rural ring east of the road in, and
  the urban ring north of the centre. Each shows the nodes by type over the
  zone rings, water, roads, lots and parks. Then it converts each SVG to a PNG
  with headless Chrome (or Edge), as suburb-plan.ps1 does.

.PARAMETER Out
  Output folder. Default: target/node-plan at the repo root.

.EXAMPLE
  powershell -File backend/tools/node-plan.ps1
#>
param(
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$backend = Split-Path $PSScriptRoot -Parent
$repo = Split-Path $backend -Parent
if (-not $Out) { $Out = Join-Path $repo "target/node-plan" }
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$env:NODE_PLAN_OUT = $Out

Get-ChildItem -Path (Join-Path $Out "*") -File -Include *.svg, *.png |
    ForEach-Object { Remove-Item $_.FullName -Force }

Push-Location $backend
try {
    cargo test --release --bin capitol-backend node_plan_svg -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw "node_plan_svg failed ($LASTEXITCODE)" }
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
    return
}
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
