# The teasers (#343): the Instagram reel, the YouTube teaser and the
# YouTube tour, cut from one library of offline shots, so every channel
# tells the same story and the whole thing can be rebuilt from the repo.
#
#   powershell -NoProfile -File tools\teaser.ps1                    # every cut
#   powershell -NoProfile -File tools\teaser.ps1 -Cut ig -Scale 0.5 # a quick draft
#   powershell -NoProfile -File tools\teaser.ps1 -Cut yt -Only wilds-range -Rerender
#
# The shots are tools\shots\shots.psd1 (a camera path, a time of day and
# any sample flags each); the cuts are tools\shots\cuts\<cut>.psd1 (the
# shots in order with their in and out points, the titles, the score).
# Each shot renders once per frame shape, portrait or landscape, into
# renders\v2\shots-<shape>\<shot>.mp4, and is kept until -Rerender. A cut
# writes renders\v2\<cut>.mp4, <cut>-notext.mp4 and <cut>-cover.jpg.
# The score is the game's own sounds (--write-sounds). Needs ffmpeg on
# the PATH. Lessons from the first teaser are in the vault note "The
# Capitol - Vertical Teaser for Socials - 2026-10-18". The .psd1 files are
# saved as UTF-8 with a BOM, or Windows PowerShell reads them as ANSI.

param(
    [ValidateSet("ig", "yt", "tour")]
    [string[]]$Cut = @("ig", "yt", "tour"),
    [string]$Exe,
    # Render only these shots (and cut nothing)
    [string[]]$Only,
    # Render shots even when their clip is already there
    [switch]$Rerender,
    # Frame size against the cut's own, for drafts: 0.5 is half
    [double]$Scale = 1
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)
$root = (Get-Location).Path

if (-not $Exe) {
    $Exe = @("target\release\capitol.exe", "target\package\release\capitol.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Exe) { throw "no capitol.exe: run play.bat or cargo build --release first" }
}
$Exe = (Resolve-Path $Exe).Path
$fps = 30
$step = 1.0 / $fps
$lead = 0.5   # game seconds before the first frame, for the world to settle
$shapes = @{ portrait = @(1080, 1920); landscape = @(3840, 2160) }

$shots = (Import-PowerShellDataFile "tools\shots\shots.psd1").Shots
$byName = @{}
foreach ($s in $shots) { $byName[$s.Name] = $s }

$suffix = if ($Scale -eq 1) { "" } else { "-x$Scale" }
$out = Join-Path $root "renders\v2"
New-Item -ItemType Directory -Force $out | Out-Null
$settings = Join-Path $env:TEMP "capitol-teaser-settings.toml"
Set-Content -Encoding ascii $settings "[graphics]`nshadows = `"high`"`n"

function Num([double]$x) { $x.ToString("0.###", [Globalization.CultureInfo]::InvariantCulture) }

function Size([string]$shape) {
    $wh = $shapes[$shape]
    # Even sizes, which yuv420p needs
    @([int]([math]::Round($wh[0] * $Scale / 2) * 2), [int]([math]::Round($wh[1] * $Scale / 2) * 2))
}

# One shot in one shape, rendered frame by frame at a fixed step and
# encoded near-losslessly, the frames thrown away (4K PNGs run to GBs).
function Render-Shot($shot, [string]$shape) {
    $dir = Join-Path $out "shots-$shape$suffix"
    New-Item -ItemType Directory -Force $dir | Out-Null
    $clip = Join-Path $dir "$($shot.Name).mp4"
    if ((Test-Path $clip) -and -not $Rerender) { return $clip }

    $w, $h = Size $shape
    if ($shape -eq "landscape" -and $shot.Size) {
        $w = [int]([math]::Round($shot.Size[0] * $Scale / 2) * 2)
        $h = [int]([math]::Round($shot.Size[1] * $Scale / 2) * 2)
    }
    $path = if ($shape -eq "portrait" -and $shot.Portrait) { $shot.Portrait } else { $shot.Path }
    $frames = Join-Path $dir "$($shot.Name)-frames"
    if (Test-Path $frames) { Remove-Item -Recurse -Force $frames }
    New-Item -ItemType Directory -Force $frames | Out-Null

    $cmd = @("--offline", "fixtures\terrain.json", "--width", $w, "--height", $h,
        "--settings-file", $settings, "--mute", "--clean", "--note= ",
        "--camera-path", "tools\shots\$path", "--time-of-day", (Num $shot.Time),
        "--frame-step", (Num $step), "--screenshot", "$frames\f.png",
        "--screenshot-at", (Num $lead), "--screenshot-every", (Num $step),
        "--exit-after", (Num ($lead + $shot.Length + 0.1)))
    if ($shot.Input) { $cmd += @("--input-script", "tools\shots\$($shot.Input)") }
    if ($shot.Args) { $cmd += $shot.Args }
    Write-Host "shot: $($shot.Name) ($shape, ${w}x$h, $($shot.Length) s)"
    # Its log goes to the console, not into the clip path this returns
    & $Exe @cmd | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "capture of $($shot.Name) failed" }
    ffmpeg -y -loglevel error -framerate $fps -i "$frames\f_%04d.png" `
        -c:v libx264 -pix_fmt yuv420p -crf 12 -preset medium $clip
    if ($LASTEXITCODE -ne 0) { throw "encoding $($shot.Name) failed" }
    Remove-Item -Recurse -Force $frames
    $clip
}

# drawtext's text, quoted for a filter script
function Quote-Text([string]$t) {
    "'" + ($t -replace "\\", "\\\\" -replace "'", "'\\\''" -replace ":", "\:" -replace "%", "\%") + "'"
}

# A title's drawtext: faded in and out over half a second, its size and
# height given at the cut's full size
function Title($t, [double]$at, [double]$until, [int]$h, [double]$k) {
    $fonts = @{ Cinzel = "Cinzel.ttf"; Sans = "SourceSans3-Regular.ttf"; Semibold = "SourceSans3-Semibold.ttf" }
    $font = $fonts[$(if ($t.Font) { $t.Font } else { "Cinzel" })]
    $color = if ($t.Color) { $t.Color } else { "0xEAC766" }
    $size = [int]($t.Size * $k)
    $y = [int]($t.Y * $k)
    $f = 0.5
    $a = Num $at; $u = Num $until
    $alpha = "if(lt(t\,$a)\,0\,if(lt(t\,$a+$f)\,(t-$a)/$f\,if(lt(t\,$u-$f)\,1\,if(lt(t\,$u)\,($u-t)/$f\,0))))"
    $sh = [math]::Max(1, [int](3 * $k))
    "drawtext=fontfile=$($font):text=$(Quote-Text $t.Text):fontsize=$($size):fontcolor=$($color):" +
    "x=(w-text_w)/2:y=$($y):shadowcolor=black@0.75:shadowx=$($sh):shadowy=$($sh):" +
    "borderw=$([math]::Max(1, [int](2 * $k))):bordercolor=black@0.45:alpha='$alpha'"
}

function Cut-One([string]$name) {
    $cut = Import-PowerShellDataFile "tools\shots\cuts\$name.psd1"
    $shape = $cut.Shape
    $w, $h = Size $shape
    $k = $w / $shapes[$shape][0]

    # The shots, rendered (or found)
    $clips = foreach ($seg in $cut.Segments) {
        $shot = $byName[$seg.Shot]
        if (-not $shot) { throw "$name names a shot shots.psd1 hasn't got: $($seg.Shot)" }
        Render-Shot $shot $shape
    }

    # Where each segment starts in the cut: each crossfade overlaps the
    # next segment onto this one's tail
    $starts = @(); $t = 0.0
    for ($i = 0; $i -lt $cut.Segments.Count; $i++) {
        $seg = $cut.Segments[$i]
        $starts += $t
        $len = $seg.Out - $seg.In
        $fade = if ($i -lt $cut.Segments.Count - 1) { Fade-Of $cut $seg } else { 0 }
        $t += $len - $fade
    }
    $total = $t
    Write-Host ("cut: {0}, {1:0.0} s, {2} segments" -f $name, $total, $cut.Segments.Count)

    $inputs = @()
    foreach ($c in $clips) { $inputs += @("-i", $c) }
    $n = $clips.Count

    # The shade behind the titles: a gradient made once as a PNG (lavfi's
    # gradients source won't fade its alpha) and looped
    $shade = Join-Path $out "shade-$name$suffix.png"
    $top = if ($cut.Shade) { $cut.Shade.Top } else { 0.3 }
    $bottom = if ($cut.Shade) { $cut.Shade.Bottom } else { 0.0 }
    $alpha = if ($cut.Shade) { $cut.Shade.Alpha } else { 0.5 }
    $ex = "255*$alpha*max(max(0\,1-Y/(H*$top+1))\,max(0\,(Y-H*(1-$bottom))/(H*$bottom+1)))"
    ffmpeg -y -loglevel error -f lavfi -i "color=black:s=${w}x${h},format=rgba" `
        -vf "geq=r=0:g=0:b=0:a='$ex'" -frames:v 1 $shade
    if ($LASTEXITCODE -ne 0) { throw "the shade failed" }
    $inputs += @("-loop", "1", "-framerate", $fps, "-t", (Num $total), "-i", $shade)
    $shadeIn = $n

    # The score: each layer a sound, looped if it's a bed, placed and faded
    $sounds = Join-Path $out "sounds"
    if (-not (Test-Path "$sounds\bell.wav")) {
        & $Exe --write-sounds $sounds | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "--write-sounds failed" }
    }
    $layers = @($cut.Score)
    $aIn = $n + 1
    foreach ($l in $layers) {
        if ($l.Loop) { $inputs += @("-stream_loop", "-1") }
        $inputs += @("-i", (Join-Path $sounds "$($l.Sound).wav"))
    }

    $g = New-Object System.Collections.Generic.List[string]
    for ($i = 0; $i -lt $n; $i++) {
        $seg = $cut.Segments[$i]
        $g.Add("[$($i):v]trim=$(Num $seg.In):$(Num $seg.Out),setpts=PTS-STARTPTS,fps=$fps,scale=${w}:${h},setsar=1[s$i];")
    }
    $prev = "s0"
    for ($i = 1; $i -lt $n; $i++) {
        $fade = Fade-Of $cut $cut.Segments[$i - 1]
        $off = $starts[$i]
        $g.Add("[$prev][s$i]xfade=transition=fade:duration=$(Num $fade):offset=$(Num $off)[x$i];")
        $prev = "x$i"
    }
    $g.Add("[$prev]format=yuv420p,fade=t=in:st=0:d=0.6,fade=t=out:st=$(Num ($total - 0.8)):d=0.8[plain];")
    $g.Add("[plain]split[pv][pt];")

    # The titled version: the shade, a dimming under the end card, titles
    $titles = @()
    for ($i = 0; $i -lt $n; $i++) {
        foreach ($t in @($cut.Segments[$i].Titles)) {
            if ($t) { $titles += Title $t ($starts[$i] + $t.At) ($starts[$i] + $t.Until) $h $k }
        }
    }
    foreach ($t in @($cut.Titles)) {
        if ($t) {
            $at = if ($t.At -lt 0) { $total + $t.At } else { $t.At }
            $titles += Title $t $at ($total + 1) $h $k
        }
    }
    $g.Add("[$($shadeIn):v]format=rgba[shade];")
    $chain = "[pt][shade]overlay=0:0:shortest=1"
    if ($cut.Card) {
        $cardAt = $total + $cut.Card.At
        $g.Add("color=black:s=${w}x${h}:d=$(Num $total):r=$fps,format=rgba,colorchannelmixer=aa=$(Num $cut.Card.Dim)," +
            "fade=t=in:st=$(Num $cardAt):d=1.0:alpha=1[dim];")
        $g.Add("$chain[sh];")
        $chain = "[sh][dim]overlay=0:0:shortest=1"
    }
    if ($titles.Count) { $chain += "," + ($titles -join ",") }
    $g.Add("$chain,format=yuv420p[tv];")

    # The score's mix
    $mix = @()
    for ($j = 0; $j -lt $layers.Count; $j++) {
        $l = $layers[$j]
        # A negative At counts back from the end
        $at = if ($l.At -lt 0) { $total + $l.At } elseif ($l.At) { $l.At } else { 0 }
        $until = if ($l.Until) { [math]::Min($l.Until, $total) } else { $total }
        $len = $until - $at
        $fi = if ($l.FadeIn) { $l.FadeIn } else { 0.01 }
        $fo = if ($l.FadeOut) { $l.FadeOut } else { 0.01 }
        $vol = if ($l.Volume) { $l.Volume } else { 1 }
        $ms = [int]($at * 1000)
        $g.Add("[$($aIn + $j):a]atrim=0:$(Num $len),asetpts=PTS-STARTPTS,aformat=sample_rates=48000:channel_layouts=stereo," +
            "volume=$(Num $vol),afade=t=in:d=$(Num $fi),afade=t=out:st=$(Num ([math]::Max(0, $len - $fo))):d=$(Num $fo)," +
            "adelay=delays=$($ms):all=1[a$j];")
        $mix += "[a$j]"
    }
    $g.Add("$($mix -join '')amix=inputs=$($layers.Count):normalize=0:duration=longest,atrim=0:$(Num $total)," +
        "afade=t=out:st=$(Num ($total - 1.5)):d=1.5,alimiter=limit=0.9[score];")
    $g.Add("[score]asplit[av][at]")

    $script = Join-Path $out "edit-$name$suffix.txt"
    # UTF-8 with no BOM: the titles may hold a middle dot, and a BOM would
    # be read as the graph's first character
    [IO.File]::WriteAllText($script, ($g -join "`n"), (New-Object Text.UTF8Encoding $false))

    $crf = if ($shape -eq "landscape") { 16 } else { 18 }
    $enc = @("-c:v", "libx264", "-crf", $crf, "-preset", "slow", "-r", $fps,
        "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart")
    $titled = Join-Path $out "$name$suffix.mp4"
    $plain = Join-Path $out "$name$suffix-notext.mp4"
    # drawtext finds the fonts by bare name from the fonts dir
    Push-Location (Join-Path $root "fonts")
    try {
        ffmpeg -y -loglevel error @inputs -filter_complex_script $script `
            -map "[tv]" -map "[at]" @enc $titled `
            -map "[pv]" -map "[av]" @enc $plain
        if ($LASTEXITCODE -ne 0) { throw "the cut $name failed (graph: $script)" }
    } finally { Pop-Location }

    $coverAt = if ($cut.Cover) { $cut.Cover } else { $total - 2 }
    ffmpeg -y -loglevel error -ss (Num $coverAt) -i $titled -frames:v 1 -q:v 2 (Join-Path $out "$name$suffix-cover.jpg")
    Write-Host "  $titled"
}

# The crossfade out of a segment into the next
function Fade-Of($cut, $seg) {
    if ($null -ne $seg.Fade) { $seg.Fade } elseif ($cut.Fade) { $cut.Fade } else { 0.5 }
}

if ($Only) {
    foreach ($o in $Only) {
        $shot = $byName[$o]
        if (-not $shot) { throw "no shot $o in shots.psd1" }
        foreach ($shape in @($shot.Shapes)) { Render-Shot $shot $shape | Out-Null }
    }
    return
}
foreach ($c in $Cut) { Cut-One $c }
Write-Host "Teasers in $out"
