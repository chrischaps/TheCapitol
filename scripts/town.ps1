<#
.SYNOPSIS
  A town: a second backend with citizens living in it, beside the usual one.

.DESCRIPTION
  The PowerShell twin of scripts/town.sh (#337), for a Windows terminal
  without Git Bash. The two share one town: the same database, port, Redis
  db and flint-client/town/, so either script can stop what the other made.

  The usual world is `cargo run` from backend/, on the database `capitol`
  and port 3000, and nothing puts citizens in it. The town has a database
  of its own, a port of its own and a Redis db of its own, so its citizens
  never claim a lot, take a vein or fill a book in the world you work in,
  and either can run without the other, or both at once.

    scripts\town.ps1 up [-Online n] [-Merchants m] [-AlwaysOn]
                            make what's missing, then run the town's backend
                            and its citizens in this terminal; Ctrl-C stops both
    scripts\town.ps1 status the town's server, and its citizens' last health
    scripts\town.ps1 reset  drop the town's database and roster, for a new town

  The first `up` creates the database, an admin for it and a roster of
  citizens. Citizens keep human hours (#276): each signs in when its habit
  says, a few real hours some days, in its own evenings, so the roster is
  sized for how many are on at once: enough of the sim's mix to keep
  -Online on (18) and enough merchants to keep -Merchants on (2), on
  average. That's about 32 citizens for each one on, so about 640 in all.
  `capitol-town habits --roster flint-client/town/town.toml` shows who's on
  when. -AlwaysOn keeps every citizen on, whatever its habit: all of them,
  so mind the pool's size. Later ups reuse the database, the admin and the
  roster, so the town carries on where it stopped. Then play in it with

    flint-client\play.bat --server http://127.0.0.1:3300

  and in the usual world with play.bat as ever. A new town knows nobody, so
  register an account on the client's form the first time. The client keeps
  one server's sign-in, so a switch asks for the password again (#338).

  What the town keeps is in flint-client/town/ (git-ignored: the roster
  holds the citizens' keys, and admin.env the admin's password), with the
  backend's log. It runs copies of the backend and capitol-town (target/town/
  and flint-client/target/town/), so a running town never locks the exe
  that `cargo run` rebuilds, nor the other way round.

  Environment:
    TOWN_PORT   the town's port (default 3300)
    TOWN_DB     its database, in the docker-compose Postgres (default capitol_town)
    TOWN_REDIS  its Redis db index (default 1; the usual world's is 0)

.EXAMPLE
  powershell -File scripts\town.ps1 up
  powershell -File scripts\town.ps1 up -Online 2 -Merchants 0
  powershell -File scripts\town.ps1 status
#>
param(
    [Parameter(Position = 0)]
    [string]$Command = "",
    [int]$Online = 18,
    [int]$Merchants = 2,
    [switch]$AlwaysOn
)

# Invoke-WebRequest's progress bar slows every health check in 5.1.
$ProgressPreference = "SilentlyContinue"

$root = Split-Path -Parent $PSScriptRoot
$port = if ($env:TOWN_PORT) { $env:TOWN_PORT } else { "3300" }
$db = if ($env:TOWN_DB) { $env:TOWN_DB } else { "capitol_town" }
$redisDb = if ($env:TOWN_REDIS) { $env:TOWN_REDIS } else { "1" }
$server = "http://127.0.0.1:$port"
$dir = Join-Path $root "flint-client\town"
$roster = Join-Path $dir "town.toml"
$adminEnv = Join-Path $dir "admin.env"
$postgres = "capitol-postgres"
$backendExe = Join-Path $root "target\town\capitol-backend.exe"
$townExe = Join-Path $root "flint-client\target\town\capitol-town.exe"

function Die([string]$message) {
    [Console]::Error.WriteLine("town: $message")
    exit 1
}

function Say([string]$message) { Write-Host "town: $message" }

# Runs a native command for its exit code alone. PowerShell 5.1 turns a
# redirected stderr line into an error record, so the preference is eased
# while it runs.
function Test-Native([scriptblock]$block) {
    $preference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try { & $block *> $null } finally { $ErrorActionPreference = $preference }
    return $LASTEXITCODE -eq 0
}

function Invoke-Psql([string]$database, [string]$sql) { # one value, no decoration
    $out = docker exec $postgres psql -U capitol -d $database -tAc $sql
    if ($LASTEXITCODE -ne 0) { Die "psql failed on $database`: $sql" }
    return "$out".Trim()
}

function Test-Healthy {
    try {
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "$server/health" | Out-Null
        return $true
    } catch {
        return $false
    }
}

function Assert-Postgres {
    if (-not (Test-Native { docker exec $postgres pg_isready -U capitol })) {
        Die "Postgres ($postgres) isn't up: run docker-compose up -d at the repo root"
    }
}

function New-Database {
    if ((Invoke-Psql capitol "select 1 from pg_database where datname = '$db'") -ne "1") {
        Say "creating the database $db"
        Invoke-Psql capitol "create database $db" | Out-Null
    }
}

# Builds a binary and copies it where only the town runs it. If the build
# fails because the usual one is running and holds the exe, the last copy
# will do, with a warning.
function Build-Copy([string]$ws, [string]$package, [string]$bin, [string]$to) {
    $built = Join-Path $ws "target\debug\$bin.exe"
    $copy = Join-Path $to "$bin.exe"
    Say "building $bin"
    Push-Location $ws
    try { cargo build -q -p $package --bin $bin } finally { Pop-Location }
    if ($LASTEXITCODE -eq 0) {
        New-Item -ItemType Directory -Force $to | Out-Null
        try {
            Copy-Item -LiteralPath $built -Destination $copy -Force -ErrorAction Stop
        } catch {
            Say "can't replace the copy of $bin (is a town running it?); running the last copy"
        }
    } elseif (Test-Path -LiteralPath $copy) {
        Say "the build of $bin failed (is the usual one running and holding its exe?); running the last copy"
    } else {
        Die "the build of $bin failed, and there's no earlier copy to run"
    }
}

# Sets environment variables for the processes started inside $block, and
# puts back what was there, so none of them stays in the caller's session.
function Invoke-WithEnv([hashtable]$vars, [scriptblock]$block) {
    $saved = @{}
    foreach ($name in $vars.Keys) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
        [Environment]::SetEnvironmentVariable($name, $vars[$name], "Process")
    }
    try { & $block } finally {
        foreach ($name in $saved.Keys) {
            [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process")
        }
    }
}

function Start-Backend {
    Build-Copy $root capitol-backend capitol-backend (Join-Path $root "target\town")
    New-Item -ItemType Directory -Force $dir | Out-Null
    $log = Join-Path $dir "backend.log"
    Say "starting the town's server on $server (its log: flint-client/town/backend.log)"
    # The variables set here win over backend/.env's: dotenvy fills only
    # what's unset. cmd puts stdout and stderr in the one log, as town.sh
    # does; Start-Process alone would want a file for each.
    $vars = @{
        DATABASE_URL = "postgres://capitol:capitol_dev@localhost:5433/$db"
        REDIS_URL = "redis://localhost:6379/$redisDb"
        HOST = "127.0.0.1"
        PORT = "$port"
    }
    $script:backend = Invoke-WithEnv $vars {
        Start-Process cmd -ArgumentList "/c `"`"$backendExe`" > `"$log`" 2>&1`"" `
            -WorkingDirectory (Join-Path $root "backend") -NoNewWindow -PassThru
    }
    $waited = 0
    while (-not (Test-Healthy)) {
        if ($script:backend.HasExited) {
            $script:backend = $null
            Get-Content -LiteralPath $log -Tail 20 | ForEach-Object { [Console]::Error.WriteLine($_) }
            Die "the town's server stopped before it was up (above, the end of its log)"
        }
        # A new database runs every migration and plants every node first.
        if ($waited -gt 0 -and $waited % 30 -eq 0) { Say "still starting ($waited s)" }
        Start-Sleep -Seconds 1
        $waited++
    }
    Say "the server is up"
}

# The town's admin mints its citizens. It's made once, on the town's own
# database, with a password nobody types.
function Read-Admin {
    if (-not (Test-Path -LiteralPath $adminEnv)) {
        $email = "clerk@town.capitol.local"
        $bytes = New-Object byte[] 18
        [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        $password = -join ($bytes | ForEach-Object { $_.ToString("x2") })
        Say "making the town's admin, $email"
        $body = @{ email = $email; password = $password; player_name = "TownClerk" } | ConvertTo-Json
        try {
            Invoke-RestMethod -Method Post -ContentType "application/json" -TimeoutSec 10 `
                -Body $body "$server/auth/register" | Out-Null
        } catch {
            Die "couldn't register the town's admin (does $email exist on $db already? try reset)"
        }
        # LF and no BOM, so town.sh can source it too.
        [IO.File]::WriteAllText($adminEnv,
            "CAPITOL_ADMIN_EMAIL=$email`nCAPITOL_ADMIN_PASSWORD=$password`n")
    }
    $admin = @{}
    foreach ($line in [IO.File]::ReadAllLines($adminEnv)) {
        if ($line -match '^([A-Z_]+)=(.*)$') { $admin[$Matches[1]] = $Matches[2] }
    }
    Invoke-Psql $db "update accounts set is_admin = true where email = '$($admin.CAPITOL_ADMIN_EMAIL)' and not is_admin" | Out-Null
    return $admin
}

function New-Citizens([hashtable]$admin, [int]$online, [int]$merchants) {
    if (Test-Path -LiteralPath $roster) {
        $count = @(Select-String -LiteralPath $roster -Pattern '^\[\[citizen\]\]').Count
        if ($count -gt 0) {
            Say "the roster has $count citizens"
            return
        }
    }
    Say "minting enough of the sim's mix to keep $online on, and enough merchants to keep $merchants on"
    Push-Location (Join-Path $root "flint-client")
    try {
        Invoke-WithEnv $admin {
            & $townExe mint --online $online --roster $roster --server $server | Select-Object -Last 1
            if ($LASTEXITCODE -ne 0) { Die "minting the citizens failed" }
            if ($merchants -ne 0) {
                & $townExe mint --online $merchants --role merchant --roster $roster --server $server | Select-Object -Last 1
                if ($LASTEXITCODE -ne 0) { Die "minting the merchants failed" }
            }
        }
    } finally { Pop-Location }
}

# Waits for a process up to a time, then ends it and anything it started.
function Stop-Town([Diagnostics.Process]$process, [int]$seconds) {
    if ($null -eq $process) { return }
    if (-not $process.WaitForExit($seconds * 1000)) {
        taskkill /T /F /PID $process.Id *> $null
    }
}

function Invoke-Up {
    if (-not (Test-Path -LiteralPath (Join-Path $root "backend\.env"))) {
        Die "backend/.env is missing: copy backend/.env.example (the town reads its JWT_SECRET)"
    }
    if (Test-Healthy) {
        Die "something already answers on $server`: another town? Stop it, or set TOWN_PORT"
    }
    Assert-Postgres
    New-Database
    Build-Copy (Join-Path $root "flint-client") capitol-core capitol-town (Join-Path $root "flint-client\target\town")
    $script:backend = $null
    $town = $null
    # Ctrl-C reaches every process on this console, the server and the
    # citizens with it, and stops this script, which runs the finally: it
    # only has to wait for them, and end any that don't stop. The waits are
    # .NET calls, which a stopping pipeline doesn't cut short.
    try {
        Start-Backend
        $admin = Read-Admin
        New-Citizens $admin $Online $Merchants
        Say "the town lives. Play in it: flint-client\play.bat --server $server"
        Say "Ctrl-C stops the citizens and the server"
        $townArgs = @("run", "--roster", "`"$roster`"", "--server", $server)
        if ($AlwaysOn) { $townArgs += "--always-on" }
        $town = Invoke-WithEnv $admin {
            Start-Process $townExe -ArgumentList $townArgs `
                -WorkingDirectory (Join-Path $root "flint-client") -NoNewWindow -PassThru
        }
        # Wait on the town; if it ends by itself (every key refused, say), stop.
        while (-not $town.HasExited) { Start-Sleep -Milliseconds 500 }
    } finally {
        Write-Host ""
        Write-Host "town: stopping: the citizens finish their steps (up to 20 s), then the server"
        Stop-Town $town 25
        Stop-Town $script:backend 10
        Write-Host "town: stopped"
    }
}

function Invoke-Status {
    if (Test-Healthy) { Say "the server answers on $server" } else { Say "nothing answers on $server" }
    if (Test-Path -LiteralPath (Join-Path $dir "town.status")) {
        & $townExe status --roster $roster
        if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $dir "town.status") }
    } else {
        Say "no citizens have run here yet"
    }
}

function Invoke-Reset {
    if (Test-Healthy) { Die "the town is running on $server`: stop it first" }
    Assert-Postgres
    Say "dropping the database $db and the roster in flint-client/town/"
    Invoke-Psql capitol "drop database if exists $db with (force)" | Out-Null
    # Only the town's own files: the roster, its keys, its health and logs.
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    # The town's sessions and refresh tokens name accounts that are gone now.
    Test-Native { docker exec capitol-redis redis-cli -n $redisDb flushdb } | Out-Null
    Say "done: the next up makes a new town"
}

switch ($Command) {
    "up" { Invoke-Up }
    "status" { Invoke-Status }
    "reset" { Invoke-Reset }
    default {
        Get-Help $PSCommandPath -Detailed
        exit 2
    }
}
