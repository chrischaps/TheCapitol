# Package the Flint client for another Windows machine (P8.7).
#
#   powershell -NoProfile -File package.ps1            # build, stage, zip
#   powershell -NoProfile -File package.ps1 -NoBuild   # re-stage the last build
#
# Writes dist\TheCapitol\ (the folder as a player gets it) and
# dist\TheCapitol-<version>.zip. The exe is built with a static C runtime,
# in its own target dir (target\package), so the zip doesn't need the
# Visual C++ redistributable and the build doesn't disturb target\release.
# The game finds its data beside the exe wherever it's started from
# (enter_game_dir in main.rs), and writes save\ and cache\
# there, so the folder must be somewhere the player can write.

param(
    [switch]$NoBuild,
    [string]$Out = "dist"
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$target = "target\package"
if (-not $NoBuild) {
    # RUSTFLAGS replaces .cargo\config.toml's rustflags, so the 8 MB stack
    # is repeated here.
    $env:RUSTFLAGS = "-C link-arg=/STACK:8388608 -C target-feature=+crt-static"
    $env:CARGO_TARGET_DIR = $target
    cargo build --release --bin capitol
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = "$target\release\capitol.exe"
if (-not (Test-Path $exe)) { throw "$exe is missing: run without -NoBuild" }

$version = (git describe --tags --always --dirty 2>$null)
if (-not $version) { $version = (Get-Date -Format "yyyyMMdd") }

$stage = Join-Path $Out "TheCapitol"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item $exe $stage
# The game's data. Only the files the game reads: no Rust, no tests.
foreach ($dir in "scenes", "schemas", "scripts", "config", "fonts", "assets", "particles", "models") {
    Copy-Item -Recurse $dir (Join-Path $stage $dir)
}
# The engine's schemas sit under the game's (main.rs loads engine\schemas
# first). Nothing else of the engine is read at run time: its shaders are
# compiled in.
New-Item -ItemType Directory -Force (Join-Path $stage "engine") | Out-Null
Copy-Item -Recurse "engine\schemas" (Join-Path $stage "engine\schemas")
# The sounds are written at first start; the credits come along.
New-Item -ItemType Directory -Force (Join-Path $stage "audio") | Out-Null
Copy-Item "audio\CREDITS.md" (Join-Path $stage "audio")
# A captured world, so the city can be seen with no server.
New-Item -ItemType Directory -Force (Join-Path $stage "fixtures") | Out-Null
Copy-Item "fixtures\terrain.json" (Join-Path $stage "fixtures")
Get-ChildItem $stage -Recurse -Filter ".gitkeep" | Remove-Item -Force

Set-Content -Encoding ascii (Join-Path $stage "Explore offline.bat") @"
@echo off
rem The city from a captured world, with no server and no login.
rem WASD moves, the wheel or R/F zoom, Q/E orbit, H comes home.
cd /d "%~dp0"
start "" capitol.exe --offline fixtures\terrain.json
"@

Set-Content -Encoding utf8 (Join-Path $stage "README.txt") @"
The Capitol: the Flint client ($version)
==========================================

The Capitol is an MMO where the economy is the game: gather, craft, trade
and build on a shared world. This is its 3D client.

PLAYING
  Run capitol.exe. Under the login form is the server it talks to: click
  "change" and give the address of a Capitol server, such as
  http://192.168.1.20:3000. Then create an account or sign in. The client remembers the server, your email and your session in
  save\session.toml.

  To pick the server from a shortcut or a terminal instead:
    capitol.exe --server http://192.168.1.20:3000
  or set the CAPITOL_SERVER environment variable.

  To see the city with no server at all, run "Explore offline.bat".

KEYS
  Click the ground to walk, a resource node to gather, a station to use
  it, a bazaar stall to see its shop.
  I inventory, C crafting, X fiber exchange, M the market, G the Gazette,
  T trade, O nearest station. Alt+click a plot for its deed.
  Wheel or R/F zoom, Q/E or right-drag orbit, H home.
  F1 or ? lists every shortcut. F10 opens the settings.

NEEDS
  Windows 10 or 11, 64-bit, and a GPU with DirectX 12 or Vulkan.
  Keep this folder somewhere you can write to (not Program Files):
  the game writes save\ and cache\ here.

  capitol.exe --help lists the command-line options.

Fonts: Source Sans 3 and Cinzel, under the SIL Open Font License (fonts\).
"@

New-Item -ItemType Directory -Force $Out | Out-Null
$zip = Join-Path $Out "TheCapitol-$version.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip
$mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "Packaged $zip ($mb MB)"
