# package-anicat-windows.ps1 - builds the Windows zip a release carries.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-anicat-windows.ps1
#
# Produces dist\Anicat-windows\ and dist\Anicat-<version.txt>-windows-x64.zip:
#   anicat.exe            the server crate, release build
#   mpv.exe               the pinned mpv build below
#   d3dcompiler_43.dll    carried beside mpv.exe by that build
#   mpv\mpv.conf          ours (server\mpv\mpv.conf); the player passes this
#                         folder to mpv as --config-dir
#   mpv\input.conf        ours, the Ctrl+number / Shift+letter bindings
#   mpv\scripts\          modernz.lua (the skin) and anicat.lua
#   mpv\script-opts\      the skin's Anicat theme
#   mpv\fonts\            the skin's icon font
#   mpv\shaders\          Anime4K, from the Mac app's copy
#   mpv\fonts.conf        from the mpv archive, same folder
#   THIRD_PARTY_NOTICES.txt, README.txt, DISCLAIMER.md
#
# Needs the toolchain from scripts\setup-windows-dev.ps1, plus 7-Zip (the mpv
# build is a .7z; `winget install 7zip.7zip`). The windows-latest runner has
# both. Export ANICAT_TMDB_PROXY first for a release: it is read at compile
# time, and without it Films and TV is missing from the build with no error.
#
# Runs on Windows PowerShell 5.1 and PowerShell 7.

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 redraws Invoke-WebRequest's progress bar per chunk,
# which slows a 30 MB download from seconds to minutes.
$ProgressPreference = 'SilentlyContinue'

# --- Pinned mpv --------------------------------------------------------------
#
# shinchiro's build (github.com/shinchiro/mpv-winbuild-cmake), the usual mpv
# for Windows and the one mpv.io links to. Its tagged-release builds live on
# SourceForge only. The GitHub releases of that repository are nightly
# snapshots of mpv's master and are pruned after about thirty days, so a URL
# pinned there 404s within a month; the release/ folder on SourceForge still
# holds 0.39.0. 0.41.0 is also the mpv the Mac build links through MPVKit.
#
# x86_64, not x86_64-v3: v3 is compiled for AVX2 and dies with an illegal
# instruction on CPUs without it, which a public download cannot rule out.
#
# To bump: pick the new mpv-<version>-x86_64.7z from
# https://sourceforge.net/projects/mpv-player-windows/files/release/, then
#   curl -fsSL -A Wget -o mpv.7z https://downloads.sourceforge.net/project/mpv-player-windows/release/mpv-<version>-x86_64.7z
#   shasum -a 256 mpv.7z            (or Get-FileHash mpv.7z on Windows)
#   bsdtar -tvf mpv.7z              (or 7z l mpv.7z: check $MpvFiles still exists)
# update the three values below, and update WINDOWS_NATIVE in
# scripts\generate-third-party-notices.py (its comment says how to re-derive
# the library list), then regenerate the notices. If SourceForge becomes
# unusable from CI, the dated mpv-x86_64-<date>-git-<hash>.7z assets on the
# GitHub releases are the fallback, with the pruning caveat above.
$MpvVersion = '0.41.0'
$MpvUrl = 'https://downloads.sourceforge.net/project/mpv-player-windows/release/mpv-0.41.0-x86_64.7z'
$MpvSha256 = 'ef86fde0959d789d77a3ad7c3c2dca51c6999695363f493a6154f2c518634c0f'
# What the archive holds besides doc\manual.pdf and mpv.com. mpv.com is the
# console wrapper, useless under a player that runs without a console.
# d3dcompiler_43.dll is the D3D shader compiler mpv's d3d11 paths fall back to
# when the system's newer one is missing. fonts.conf is fontconfig's
# configuration, which mpv reads from its config dir: without it libass's
# font fallback breaks and subtitles render in a substitute font, silently.
$MpvFiles = @('mpv.exe', 'd3dcompiler_43.dll', 'mpv\fonts.conf')

# --- Paths -------------------------------------------------------------------

$Root = Split-Path -Parent $PSScriptRoot
$Version = (Get-Content -Raw (Join-Path $Root 'version.txt')).Trim()
$Dist = Join-Path $Root 'dist'
$Stage = Join-Path $Dist 'Anicat-windows'
$Cache = Join-Path $Dist 'cache'
$Zip = Join-Path $Dist "Anicat-$Version-windows-x64.zip"
$ServerDir = Join-Path $Root 'server'
$Notices = Join-Path $Root 'AnicatApple\Sources\AnicatUI\Resources\Legal\THIRD_PARTY_NOTICES.txt'

function Fail($message) {
    Write-Host "package-anicat-windows: $message" -ForegroundColor Red
    exit 1
}

function Find-SevenZip {
    $onPath = Get-Command 7z -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if ($base) {
            $candidate = Join-Path $base '7-Zip\7z.exe'
            if (Test-Path $candidate) { return $candidate }
        }
    }
    return $null
}

# Checked before the build, not after it: a missing 7-Zip found only once
# cargo has spent ten minutes compiling reads as a download problem.
$SevenZip = Find-SevenZip
if (-not $SevenZip) {
    Fail "7-Zip not found. The mpv build ships as a .7z; install it with 'winget install 7zip.7zip' and run this again."
}
$MpvDir = Join-Path $ServerDir 'mpv'
$MpvConf = Join-Path $MpvDir 'mpv.conf'
if (-not (Test-Path $MpvConf)) { Fail "missing $MpvConf" }
# The skin and the shaders are not decoration: without scripts\ there is no
# on-screen controller at all (mpv.conf sets osc=no for it), and without
# shaders\ every Anime4K toggle fails silently at the glsl-shaders write.
foreach ($needed in @('input.conf', 'scripts\modernz.lua', 'scripts\anicat.lua', 'script-opts\modernz.conf')) {
    if (-not (Test-Path (Join-Path $MpvDir $needed))) { Fail "missing $(Join-Path $MpvDir $needed)" }
}
$ShaderSrc = Join-Path $Root 'AnicatApple\Sources\AnicatUI\Resources\Shaders'
if (-not (Test-Path $ShaderSrc)) { Fail "missing $ShaderSrc (the Anime4K shaders the player loads)" }
if (-not (Test-Path $Notices)) { Fail "missing $Notices" }
if (-not $env:ANICAT_TMDB_PROXY) {
    Write-Host 'package-anicat-windows: ANICAT_TMDB_PROXY is not set; this build will have no Films and TV.' -ForegroundColor Yellow
}

# --- Build -------------------------------------------------------------------

# aws-lc-sys assembles its x86_64 code with NASM, and a build with no `nasm`
# on PATH failed at "NASM command not found" after three minutes of
# compiling. The NASM installer does not add itself to PATH, and a shell
# opened before setup-windows-dev.ps1 changed PATH never sees it either. The
# crate ships the assembled objects; use those rather than fail.
if (-not (Get-Command nasm -ErrorAction SilentlyContinue) -and -not $env:AWS_LC_SYS_PREBUILT_NASM) {
    Write-Host "    nasm not on PATH; using aws-lc-sys's prebuilt NASM objects"
    $env:AWS_LC_SYS_PREBUILT_NASM = '1'
}

Write-Host "==> cargo build --release --locked (Anicat $Version)"
Push-Location $ServerDir
try {
    # --locked: a release must build the dependency set the notices were
    # generated from, not whatever cargo would resolve today.
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { Fail "cargo build failed (exit code $LASTEXITCODE)" }
} finally {
    Pop-Location
}
$Exe = Join-Path $ServerDir 'target\release\anicat.exe'
if (-not (Test-Path $Exe)) { Fail "cargo reported success but $Exe does not exist" }

# --- mpv ---------------------------------------------------------------------

New-Item -ItemType Directory -Force -Path $Cache | Out-Null
$Archive = Join-Path $Cache "mpv-$MpvVersion-x86_64.7z"

function Test-Sha256($path, $expected) {
    if (-not (Test-Path $path)) { return $false }
    return ((Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant() -eq $expected)
}

if (Test-Sha256 $Archive $MpvSha256) {
    Write-Host "==> mpv $MpvVersion already downloaded"
} else {
    Write-Host "==> Downloading mpv $MpvVersion"
    # Windows PowerShell 5.1 negotiates TLS 1.0 by default, which SourceForge
    # refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    # -UserAgent Wget: SourceForge answers a browser-like user agent, which
    # Invoke-WebRequest's default is, with a 130 KB HTML "your download will
    # start" page and a 200 status instead of the archive (measured
    # 2026-09-12). The hash check below would catch it, but only as a
    # mismatch that explains nothing.
    Invoke-WebRequest -Uri $MpvUrl -OutFile $Archive -UserAgent 'Wget' -UseBasicParsing
    $actual = (Get-FileHash -Algorithm SHA256 -Path $Archive).Hash.ToLowerInvariant()
    if ($actual -ne $MpvSha256) {
        $size = (Get-Item $Archive).Length
        Remove-Item -Force $Archive
        Fail ("mpv archive hash mismatch.`n  expected $MpvSha256`n  actual   $actual ($size bytes)`n" +
            "A size far below 30 MB usually means SourceForge served an HTML page instead of the file. " +
            "Otherwise the file changed upstream: do not bump the hash without checking why.")
    }
}

# --- Stage -------------------------------------------------------------------

Write-Host "==> Staging $Stage"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'mpv') | Out-Null

# Extracted into a scratch folder and copied file by file, never extracted
# straight into the stage: the archive has its own mpv\ folder, which is the
# same folder our mpv.conf goes in. Extracting over it after the copy drops
# nothing today, but a later archive carrying an mpv.conf of its own would
# silently replace ours, and copying ours over a freshly extracted folder
# the other way round is one refactor away from losing fonts.conf.
$MpvScratch = Join-Path $Cache "mpv-$MpvVersion-extract"
if (Test-Path $MpvScratch) { Remove-Item -Recurse -Force $MpvScratch }
& $SevenZip x $Archive "-o$MpvScratch" -y | Out-Null
if ($LASTEXITCODE -ne 0) { Fail "7-Zip could not extract $Archive (exit code $LASTEXITCODE)" }
foreach ($file in $MpvFiles) {
    $source = Join-Path $MpvScratch $file
    if (-not (Test-Path $source)) { Fail "the mpv archive no longer contains $file; re-check its listing (see the bump notes at the top)" }
    Copy-Item -Force $source (Join-Path $Stage $file)
}
Remove-Item -Recurse -Force $MpvScratch

Copy-Item -Force $Exe (Join-Path $Stage 'anicat.exe')
Copy-Item -Force $MpvConf (Join-Path $Stage 'mpv\mpv.conf')
Copy-Item -Force (Join-Path $MpvDir 'input.conf') (Join-Path $Stage 'mpv\input.conf')
foreach ($folder in @('scripts', 'script-opts', 'fonts')) {
    $source = Join-Path $MpvDir $folder
    if (Test-Path $source) {
        Copy-Item -Recurse -Force $source (Join-Path $Stage 'mpv')
    }
}
# Only the .glsl files: the Mac's Shaders folder also carries a compiled
# metallib folder for the SwiftUI shaders, which mpv has no use for.
$ShaderDest = Join-Path $Stage 'mpv\shaders'
New-Item -ItemType Directory -Force -Path $ShaderDest | Out-Null
Copy-Item -Force (Join-Path $ShaderSrc '*.glsl') $ShaderDest
# The committed file, not a fresh generation: it is what --check in
# publish-release.sh holds the Mac release to, so both platforms of one tag
# ship the same text.
Copy-Item -Force $Notices (Join-Path $Stage 'THIRD_PARTY_NOTICES.txt')
Copy-Item -Force (Join-Path $ServerDir 'README.txt') (Join-Path $Stage 'README.txt')
$Disclaimer = Join-Path $Root 'DISCLAIMER.md'
if (Test-Path $Disclaimer) { Copy-Item -Force $Disclaimer (Join-Path $Stage 'DISCLAIMER.md') }

# --- Zip ---------------------------------------------------------------------

Write-Host "==> Zipping $Zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
# ZipFile rather than Compress-Archive: Windows PowerShell 5.1's
# Compress-Archive writes entry names with backslashes, which unzip on macOS
# and Linux turn into files literally named "mpv\mpv.conf" at the top level.
# ZipFile writes forward slashes under PowerShell 7, which is what the release
# workflow runs this with; under 5.1 it depends on the .NET Framework version,
# which matters only for a zip made by hand and opened off Windows.
# Windows PowerShell 5.1 does not load ZipFile's assembly on its own. On
# PowerShell 7 the type is already there and loading the assembly by that name
# can throw, which under 'Stop' would end the run after the whole cargo build.
if ($PSVersionTable.PSVersion.Major -lt 6) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
}
[System.IO.Compression.ZipFile]::CreateFromDirectory(
    $Stage, $Zip, [System.IO.Compression.CompressionLevel]::Optimal, $false)

$ZipItem = Get-Item $Zip
$Hash = (Get-FileHash -Algorithm SHA256 -Path $Zip).Hash.ToLowerInvariant()
Write-Host ''
Write-Host "Anicat $Version for Windows"
Write-Host ("  {0}" -f $ZipItem.FullName)
Write-Host ("  {0:N1} MB" -f ($ZipItem.Length / 1MB))
Write-Host "  sha256 $Hash"
