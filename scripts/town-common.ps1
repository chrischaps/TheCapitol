# What scripts\town.ps1 and scripts\balance.ps1 share: a backend of its own on
# a database, port and Redis db of its own, its admin, its citizens and
# copies of the binaries. Dot-source it after setting:
#
#   $root $port $db $redisDb $server  the repo, and the server's place
#   $dir $roster $adminEnv            where its files go
#   $backendExe $townExe              where the copies of the binaries go
#   $sayAs                            the word each line starts with
#   $backendEnv                       (optional) more variables for the server
#   $sayLog                           (optional) a file each line goes to too,
#                                     in UTF-8, however the script was started

$postgres = "capitol-postgres"

function Die([string]$message) {
    [Console]::Error.WriteLine("$sayAs`: $message")
    Write-SayLog "$sayAs`: $message"
    exit 1
}

function Say([string]$message) {
    Write-Host "$sayAs`: $message"
    Write-SayLog "$sayAs`: $message"
}

# Started with *> under PowerShell 5.1, a script's output is UTF-16, which
# grep and tail don't read (#554). $sayLog, when set, is a copy in UTF-8.
function Write-SayLog([string]$line) {
    if ($sayLog) {
        [IO.File]::AppendAllText($sayLog, "$(Get-Date -Format 'HH:mm:ss') $line`n")
    }
}

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

# Drops the database and flushes the Redis db: its sessions and refresh
# tokens name accounts that are gone now.
function Remove-Database {
    Invoke-Psql capitol "drop database if exists $db with (force)" | Out-Null
    Test-Native { docker exec capitol-redis redis-cli -n $redisDb flushdb } | Out-Null
}

# Builds a binary and copies it where only this script runs it. If the
# build fails because the usual one is running and holds the exe, the last
# copy will do, with a warning. $profile is cargo's: dev unless given.
function Build-Copy([string]$ws, [string]$package, [string]$bin, [string]$to, [string]$profile = "dev") {
    $out = if ($profile -eq "dev") { "debug" } else { $profile }
    $built = Join-Path $ws "target\$out\$bin.exe"
    $copy = Join-Path $to "$bin.exe"
    Say "building $bin"
    Push-Location $ws
    try { cargo build -q -p $package --bin $bin --profile $profile } finally { Pop-Location }
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

# Starts the server from the copy, logging to $dir\backend.log, and waits
# until it answers. The process is $script:backend.
function Start-Backend {
    New-Item -ItemType Directory -Force $dir | Out-Null
    $log = Join-Path $dir "backend.log"
    Say "starting the server on $server (its log: $log)"
    # The variables set here win over backend/.env's: dotenvy fills only
    # what's unset. cmd puts stdout and stderr in the one log, as town.sh
    # does; Start-Process alone would want a file for each.
    $vars = @{
        DATABASE_URL = "postgres://capitol:capitol_dev@localhost:5433/$db"
        REDIS_URL = "redis://localhost:6379/$redisDb"
        HOST = "127.0.0.1"
        PORT = "$port"
    }
    if ($backendEnv) { foreach ($name in $backendEnv.Keys) { $vars[$name] = $backendEnv[$name] } }
    $script:backend = Invoke-WithEnv $vars {
        Start-Process cmd -ArgumentList "/c `"`"$backendExe`" > `"$log`" 2>&1`"" `
            -WorkingDirectory (Join-Path $root "backend") -NoNewWindow -PassThru
    }
    $waited = 0
    while (-not (Test-Healthy)) {
        if ($script:backend.HasExited) {
            $script:backend = $null
            Get-Content -LiteralPath $log -Tail 20 | ForEach-Object { [Console]::Error.WriteLine($_) }
            Die "the server stopped before it was up (above, the end of its log)"
        }
        # A new database runs every migration and plants every node first.
        if ($waited -gt 0 -and $waited % 30 -eq 0) { Say "still starting ($waited s)" }
        Start-Sleep -Seconds 1
        $waited++
    }
    Say "the server is up"
}

# The admin mints the citizens and reads the books. It's made once, on the
# database, with a password nobody types.
function Read-Admin {
    if (-not (Test-Path -LiteralPath $adminEnv)) {
        $email = "clerk@town.capitol.local"
        $bytes = New-Object byte[] 18
        [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        $password = -join ($bytes | ForEach-Object { $_.ToString("x2") })
        Say "making the admin, $email"
        $body = @{ email = $email; password = $password; player_name = "TownClerk" } | ConvertTo-Json
        try {
            Invoke-RestMethod -Method Post -ContentType "application/json" -TimeoutSec 10 `
                -Body $body "$server/auth/register" | Out-Null
        } catch {
            Die "couldn't register the admin (does $email exist on $db already? try reset)"
        }
        # LF and no BOM, so town.sh can source it too.
        New-Item -ItemType Directory -Force (Split-Path -Parent $adminEnv) | Out-Null
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

# Mints citizens onto the roster: -Online keeps that many on, on average,
# by their habits; -Count makes exactly that many. $merchants more are
# born to trade, trading favourites (Callings K2.2): anyone leaning to
# trading trades once it has its float, so 0 still trades.
function New-Citizens([hashtable]$admin, [string]$size, [int]$citizens, [int]$merchants) {
    Push-Location (Join-Path $root "flint-client")
    try {
        Invoke-WithEnv $admin {
            & $townExe mint $size $citizens --roster $roster --server $server | Select-Object -Last 1
            if ($LASTEXITCODE -ne 0) { Die "minting the citizens failed" }
            if ($merchants -ne 0) {
                & $townExe mint $size $merchants --role merchant --roster $roster --server $server | Select-Object -Last 1
                if ($LASTEXITCODE -ne 0) { Die "minting the trading favourites failed" }
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
