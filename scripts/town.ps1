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
  -Online on (18) and enough merchants, trading favourites, to keep
  -Merchants on (2), on average. Since Callings K2.2 anyone leaning to
  trading makes a market once it has its float, so -Merchants 0 still
  trades. That's about 32 citizens for each one on, so about 640 in all.
  `capitol-town habits --roster flint-client/town/town.toml` shows who's on
  when. -AlwaysOn keeps every citizen on, whatever its habit: all of them,
  so mind the pool's size. Later ups reuse the database, the admin and the
  roster, so the town carries on where it stopped. Then play in it with

    flint-client\play.bat --server http://127.0.0.1:3300

  and in the usual world with play.bat as ever. A new town knows nobody, so
  register an account on the client's form the first time. The client keeps
  a sign-in for each server, so switching back and forth needs none (#338).

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
$backendExe = Join-Path $root "target\town\capitol-backend.exe"
$townExe = Join-Path $root "flint-client\target\town\capitol-town.exe"
$sayAs = "town"

. (Join-Path $PSScriptRoot "town-common.ps1")

function New-TownCitizens([hashtable]$admin, [int]$online, [int]$merchants) {
    if (Test-Path -LiteralPath $roster) {
        $count = @(Select-String -LiteralPath $roster -Pattern '^\[\[citizen\]\]').Count
        if ($count -gt 0) {
            Say "the roster has $count citizens"
            return
        }
    }
    Say "minting enough of the sim's mix to keep $online on, and enough trading favourites to keep $merchants on"
    New-Citizens $admin "--online" $online $merchants
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
        Build-Copy $root capitol-backend capitol-backend (Join-Path $root "target\town")
        Start-Backend
        $admin = Read-Admin
        New-TownCitizens $admin $Online $Merchants
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
    Remove-Database
    # Only the town's own files: the roster, its keys, its health and logs.
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
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
