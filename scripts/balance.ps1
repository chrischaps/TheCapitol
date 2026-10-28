<#
.SYNOPSIS
  A balance run (D6.3): the same town lived under two presets of tunables,
  and what changed. Or one town under one preset, for a soak.

.DESCRIPTION
  Each preset gets a new town: a database, server, roster and admin of its
  own, the same number of citizens, and a soak of -Hours of the day's clock
  with every citizen on. The soak sets the preset before it reads the
  books, so the whole run is under it, and every knob the preset doesn't
  name is at its default. Then capitol-town compare holds the two up
  against each other: the knobs, whether each one's books balanced, the
  citizens' Strands by trade, the Gini, the faucets and sinks by type, and
  prices against their targets.

    scripts\balance.ps1 A.toml [B.toml] [-Hours 2] [-Citizens 20] [-Merchants 2] [-Scale 10]
                         [-StartAt 9] [-Roster R.toml | -Roster RA.toml,RB.toml]

  With one preset there's one town, a, and no comparison: a soak of
  -Citizens under that preset, its report and balance in a\.

  -Merchants is how many more are born to trade, trading favourites
  (Callings K2.2). Anyone leaning to trading makes a market once it has
  its float, so -Merchants 0 still trades.

  The two towns live at the same time. The citizens' day runs on the real
  clock (a game day is two real hours), so two runs one after the other
  would live different hours of it: one at the market, the other asleep.
  At once, they live the same hours on the same machine, in half the time.

  -StartAt picks the hours: each soak sets its server's day clock to that
  game hour (9, or 13:30) as it starts, before any citizen signs in, so
  the run lives from there whenever it's started (#427). Runs started
  apart, even on different days, then live the same hours and compare.
  Without it, the hours are the real clock's; -Hours 24 lives a whole day
  either way. The servers run with DEV_CLOCK=on, which lets an admin set
  the day clock; it moves the hour of the day alone, never timestamps.

  A preset is TOML, one knob a line, as GET /admin/tunables keys them:

    property_tax_rate = 0.2
    "yield.grass" = 1.5          # a family's knob, by its id
    [deed_price]
    urban = 2.0                  # or under its family's table

  scripts\presets\ has two to start from. An empty file is every default.

  A preset holds the server's knobs. The town's own, the roster's tables
  ([callings] mix, [trading] line, [hiring] line, [patrons] line, [town]
  layers and voice), go in -Roster: TOML that starts each town's roster
  before its citizens are minted, so the mix deals their callings and the
  lines are theirs. One file is for every town; two are a's and b's, to
  hold two rosters up against each other under the same preset:

    [trading]
    line = 3000
    [callings]
    mix = { specialist = 0.4, leaner = 0.4, all-rounder = 0.2 }

  The runs live in flint-client\balance\<when>\ (git-ignored): a and b,
  each with its roster, its server's log, its soak's output (soak.log) and
  its report, books and balance, and the comparison beside them as
  balance.md. run.log there is this script's own lines, in UTF-8 however
  it was started. Two runs started in the same second get <when> and
  <when>-2: the folder is taken whole or not at all (#554). Two hours of
  the day's clock is ten real minutes, plus the servers' first start.

  The towns have databases, ports and Redis dbs of their own, so they
  never touch the usual world or the town, and can run beside them. Two
  balance runs at once each need their own BALANCE_PORT, BALANCE_DB and
  BALANCE_REDIS, two apart. A long run is best started detached, so
  nothing reaps it with the shell that started it:

    Start-Process powershell -WindowStyle Hidden -ArgumentList '-File scripts\balance.ps1 a.toml -Hours 24'

  The servers are the backend's soak build, optimised but with the debug
  assertions that let SIM_TICK_SCALE run, and run with RATE_LIMITS=off,
  since citizens at ten times the pace go over every limit.

  Environment:
    BALANCE_PORT   town a's port; b's is the next (default 3400)
    BALANCE_DB     the databases' prefix, _a and _b, each dropped before
                   its run (default capitol_balance)
    BALANCE_REDIS  town a's Redis db index; b's is the next, each flushed
                   before its run (default 2)
    BALANCE_TOWN_A a capitol-town exe for town a in place of this checkout's,
                   to hold two builds up against each other under the same
                   preset (say master's in a and a branch's in b, K3.2).
                   Town b, and compare, run this checkout's build
    BALANCE_LOG_MB each citizen's decision log, in MB, to town.decisions/
                   beside its roster (default 0: none)

  Exits 1 if a run's books drifted.

.EXAMPLE
  powershell -File scripts\balance.ps1 scripts\presets\tax-2pct.toml scripts\presets\tax-20pct.toml
  powershell -File scripts\balance.ps1 a.toml b.toml -Hours 0.5 -Citizens 8 -Merchants 1
  powershell -File scripts\balance.ps1 scripts\presets\tax-2pct.toml -Citizens 200 -Roster line-3000.toml
  powershell -File scripts\balance.ps1 a.toml b.toml -Hours 4 -StartAt 8     # the working day, from 08:00
#>
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [string]$A,
    [Parameter(Position = 1)]
    [string]$B,
    [double]$Hours = 2,
    [int]$Citizens = 20,
    [int]$Merchants = 2,
    [int]$Scale = 10,
    [string[]]$Roster = @(),
    [string]$StartAt
)

# Invoke-WebRequest's progress bar slows every health check in 5.1.
$ProgressPreference = "SilentlyContinue"

$root = Split-Path -Parent $PSScriptRoot
$basePort = if ($env:BALANCE_PORT) { [int]$env:BALANCE_PORT } else { 3400 }
$dbPrefix = if ($env:BALANCE_DB) { $env:BALANCE_DB } else { "capitol_balance" }
$baseRedis = if ($env:BALANCE_REDIS) { [int]$env:BALANCE_REDIS } else { 2 }
$runs = $null
# PowerShell's names aren't cased: town-common's $roster is the param's too
# and under powershell -File, a,b comes as one string
$rosterFiles = @($Roster | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$backendExe = Join-Path $root "target\balance\capitol-backend.exe"
$townExe = Join-Path $root "flint-client\target\balance\capitol-town.exe"
$townExeA = if ($env:BALANCE_TOWN_A) { (Resolve-Path -LiteralPath $env:BALANCE_TOWN_A).Path } else { $townExe }
$logMb = if ($env:BALANCE_LOG_MB) { [int]$env:BALANCE_LOG_MB } else { 0 }
$backendEnv = @{ SIM_TICK_SCALE = "$Scale"; RATE_LIMITS = "off"; DEV_CLOCK = "on" }
$sayAs = "balance"
# town-common.ps1 works on one town at a time: Use-Town points it at one
$port = $null; $db = $null; $redisDb = $null; $server = $null
$dir = $runs; $roster = $null; $adminEnv = $null

. (Join-Path $PSScriptRoot "town-common.ps1")

function Use-Town([hashtable]$town) {
    $script:port = $town.Port
    $script:db = $town.Db
    $script:redisDb = $town.Redis
    $script:server = "http://127.0.0.1:$($town.Port)"
    $script:dir = Join-Path $runs $town.Name
    $script:roster = Join-Path $script:dir "town.toml"
    $script:adminEnv = Join-Path $script:dir "admin.env"
}

# Takes a new folder for this run, flint-client\balance\<when>, or <when>-2
# and on if another run took that one in the same second. The folder is
# ours once we've made its run.log: creating a file is all or nothing, so
# of two runs only one can (#554).
function New-RunFolder {
    $parent = Join-Path $root "flint-client\balance"
    $when = Get-Date -Format "yyyyMMdd-HHmmss"
    for ($n = 1; $n -le 50; $n++) {
        $name = if ($n -eq 1) { $when } else { "$when-$n" }
        $folder = Join-Path $parent $name
        New-Item -ItemType Directory -Force $folder | Out-Null
        try {
            $mark = [IO.File]::Open((Join-Path $folder "run.log"), [IO.FileMode]::CreateNew)
            $mark.Close()
            return $folder
        } catch [IO.IOException] {
            continue
        }
    }
    Die "couldn't take a folder in $parent for this run"
}

# Makes a new town for one preset: its database, server, admin and citizens.
function New-BalanceTown([hashtable]$town) {
    Use-Town $town
    New-Item -ItemType Directory -Force $dir | Out-Null
    Say "town $($town.Name), for $(Split-Path -Leaf $town.Preset), on $server"
    Remove-Database
    New-Database
    $script:backend = $null
    Start-Backend
    $town.Backend = $script:backend
    $town.Admin = Read-Admin
    if ($town.Roster) {
        # Before the mint, so the mix deals their callings and each citizen takes the lines
        $tables = [IO.File]::ReadAllText($town.Roster)
        [IO.File]::WriteAllText($roster, $tables.TrimEnd() + "`n")
        Say "the roster starts from $(Split-Path -Leaf $town.Roster)"
    }
    Say "minting $Citizens of the sim's mix and $Merchants trading favourites"
    New-Citizens $town.Admin "--count" $Citizens $Merchants | Out-Host
}

# Starts the town's soak, its output to soak.log beside its roster.
function Start-Soak([hashtable]$town) {
    Use-Town $town
    $soakArgs = @("soak", "--hours", "$Hours", "--always-on", "--log-mb", "$logMb",
        "--tunables", "`"$($town.Preset)`"", "--roster", "`"$roster`"", "--server", $server)
    if ($StartAt) { $soakArgs += @("--start-at", $StartAt) }
    $town.Log = Join-Path $dir "soak.log"
    $town.Soak = Invoke-WithEnv $town.Admin {
        Start-Process $town.Exe -ArgumentList $soakArgs -WorkingDirectory (Join-Path $root "flint-client") `
            -RedirectStandardOutput $town.Log -RedirectStandardError (Join-Path $dir "soak.err") -NoNewWindow -PassThru
    }
    # Without its handle held, a process started this way forgets its exit code
    $null = $town.Soak.Handle
}

# The soak's last health line: the game clock and who's working
function Get-Health([hashtable]$town) {
    if (-not (Test-Path -LiteralPath $town.Log)) { return "starting" }
    $line = Get-Content -LiteralPath $town.Log -Tail 60 -Encoding UTF8 | Where-Object { $_ -match '^\S+ capitol-town' } |
        Select-Object -Last 1
    if ($line) { return ($line -replace '^.*?UTC\s+\S+\s+', '') } else { return "starting" }
}

$presets = @(@($A, $B) | Where-Object { $_ })
foreach ($file in $presets + $rosterFiles) {
    if (-not (Test-Path -LiteralPath $file)) { Die "no file at $file" }
}
if ($rosterFiles.Count -gt $presets.Count) { Die "-Roster gives $($rosterFiles.Count) rosters for $($presets.Count) town(s)" }
$towns = @(for ($i = 0; $i -lt $presets.Count; $i++) {
    $name = "ab"[$i]
    $rosterOf = if ($rosterFiles.Count -eq 0) { $null } elseif ($rosterFiles.Count -eq 1) { $rosterFiles[0] } else { $rosterFiles[$i] }
    @{
        Name = "$name"; Preset = (Resolve-Path -LiteralPath $presets[$i]).Path
        Port = $basePort + $i; Db = "${dbPrefix}_$name"; Redis = $baseRedis + $i
        Exe = if ($i -eq 0) { $townExeA } else { $townExe }
        Roster = if ($rosterOf) { (Resolve-Path -LiteralPath $rosterOf).Path } else { $null }
    }
})
if (-not (Test-Path -LiteralPath (Join-Path $root "backend\.env"))) {
    Die "backend/.env is missing: copy backend/.env.example (the servers read its JWT_SECRET)"
}
foreach ($town in $towns) {
    # Not Use-Town: the run has no folder yet
    $server = "http://127.0.0.1:$($town.Port)"
    if (Test-Healthy) { Die "something already answers on $server`: another balance run? Stop it, or set BALANCE_PORT" }
}
Assert-Postgres
$runs = New-RunFolder
$sayLog = Join-Path $runs "run.log"
Say "this run's folder is $runs"
Build-Copy $root capitol-backend capitol-backend (Join-Path $root "target\balance") soak
Build-Copy (Join-Path $root "flint-client") capitol-core capitol-town (Join-Path $root "flint-client\target\balance")

# Ctrl-C reaches every process on this console, and stops this script,
# which runs the finally: it waits for the soaks, then ends the servers.
$drifted = $false
try {
    foreach ($town in $towns) { New-BalanceTown $town }
    if ($towns.Count -gt 1) {
        Say "both soaks start now, so they live the same hours: $Hours of the day's clock at ${Scale}x$(if ($StartAt) { ", from $StartAt" })"
    } else {
        Say "the soak starts now: $Hours of the day's clock at ${Scale}x$(if ($StartAt) { ", from $StartAt" })"
    }
    foreach ($town in $towns) { Start-Soak $town }
    Say "the output is in $(Join-Path $runs 'a\soak.log')$(if ($towns.Count -gt 1) { " and b's" })"
    $next = (Get-Date).AddSeconds(60)
    while (@($towns | Where-Object { -not $_.Soak.HasExited }).Count -gt 0) {
        if ((Get-Date) -ge $next) {
            foreach ($town in $towns) { Say "$($town.Name): $(Get-Health $town)" }
            $next = $next.AddSeconds(60)
        }
        Start-Sleep -Seconds 2
    }
    foreach ($town in $towns) {
        $town.Soak.WaitForExit()
        Get-Content -LiteralPath $town.Log -Encoding UTF8 | Where-Object { $_ -match '^capitol-town: (Strands|what a balance)' } |
            ForEach-Object { Say "$($town.Name): $($_ -replace '^capitol-town: ', '')" }
        if ($town.Soak.ExitCode -ne 0) { $drifted = $true }
    }
} finally {
    foreach ($town in $towns) { Stop-Town $town.Soak 25 }
    # Their databases go before the next run, so they needn't stop kindly
    foreach ($town in $towns) { Stop-Town $town.Backend 0 }
}

$balances = @(foreach ($town in $towns) {
    $found = Get-ChildItem -LiteralPath (Join-Path $runs $town.Name) -Filter "*.balance.json" | Sort-Object LastWriteTime | Select-Object -Last 1
    if ($null -eq $found) { Die "town $($town.Name) wrote no balance: see its soak.log, soak.err and backend.log in $runs" }
    $found.FullName
})
if ($balances.Count -gt 1) {
    $comparison = Join-Path $runs "balance.md"
    & $townExe compare $balances[0] $balances[1] --out $comparison | Out-Null
    if ($LASTEXITCODE -ne 0) { $drifted = $true }
    Say "the comparison is in $comparison"
} else {
    Say "one town, so no comparison: its balance is $($balances[0])"
}
if ($drifted) {
    Say "a run's books drifted: see its soak's report in $runs"
    exit 1
}
