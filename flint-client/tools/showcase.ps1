# The showcase capture for chaps.dev (P8.7): stills of the city from its
# fixed shots and a short clip, all offline from fixtures\terrain.json, so
# they come out the same every time and need no server.
#
#   powershell -NoProfile -File tools\showcase.ps1                # stills and clip
#   powershell -NoProfile -File tools\showcase.ps1 -NoClip
#
# Writes renders\showcase\*.png and renders\showcase\capitol.mp4 (renders\
# is gitignored). The clip needs ffmpeg on the PATH. The exe defaults to
# the packaged build (package.ps1), else target\release. Captures read
# a settings file of their own (the defaults), never save\settings.toml.

param(
    [string]$Exe,
    [int]$Width = 1920,
    [int]$Height = 1080,
    [switch]$NoClip
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)

if (-not $Exe) {
    $Exe = @("target\package\release\capitol.exe", "target\release\capitol.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Exe) { throw "no capitol.exe: run package.ps1 or play.bat first" }
}
$out = "renders\showcase"
New-Item -ItemType Directory -Force $out | Out-Null
# The best shadows. The clip's move is keyframed (--camera-path, issue
# #92), so the camera settings don't touch it.
$settings = Join-Path $env:TEMP "capitol-showcase-settings.toml"
Set-Content -Encoding ascii $settings "[graphics]`nshadows = `"high`"`n"
$common = @("--offline", "fixtures\terrain.json", "--width", $Width, "--height", $Height,
    "--settings-file", $settings, "--mute", "--clean", "--controls", "click")

# The shots, in the order a page would show them: the tower, then out
# through the rings, then the whole map.
$shots = "capitol", "crown", "bridge", "urban", "suburb", "from-rural", "from-wilderness", "range", "district", "map"
foreach ($view in $shots) {
    Write-Host "still: $view"
    & $Exe @common --view $view --screenshot "$out\$view.png" --screenshot-at 4 --exit-after 4.5
    if ($LASTEXITCODE -ne 0) { throw "capture of $view failed" }
}

if (-not $NoClip) {
    $frames = "$out\clip"
    if (Test-Path $frames) { Remove-Item -Recurse -Force $frames }
    New-Item -ItemType Directory -Force $frames | Out-Null
    Write-Host "clip: 11 s at 30 fps"
    & $Exe @common --camera-path tools\showcase-path.txt `
        --frame-step 0.033333333 --screenshot "$frames\f.png" `
        --screenshot-at 0.5 --screenshot-every 0.033333333 --exit-after 11.5
    if ($LASTEXITCODE -ne 0) { throw "clip capture failed" }
    ffmpeg -y -loglevel error -framerate 30 -i "$frames\f_%04d.png" `
        -c:v libx264 -pix_fmt yuv420p -crf 18 -preset slow -movflags +faststart "$out\capitol.mp4"
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed" }
    Remove-Item -Recurse -Force $frames
}
Write-Host "Showcase in $out"
