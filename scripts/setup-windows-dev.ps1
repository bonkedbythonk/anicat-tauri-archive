# setup-windows-dev.ps1 - installs what `cd server; cargo build --release`
# needs on a fresh Windows 10/11 machine, and nothing else.
#
# Usage, from a normal (non-admin) PowerShell in the repo root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-windows-dev.ps1
#
# Safe to run again: every step checks first and says whether it installed
# something or found it already there. winget may still raise a UAC prompt
# for the Build Tools and CMake installers; that is the installer asking,
# not this script.
#
# This is also the description of the toolchain the CI `windows` job
# installs. Change one, change the other.
#
# Why each piece is here:
#   rustup           the Rust toolchain, on the MSVC target.
#   VS Build Tools   link.exe and the Windows SDK. Rust's MSVC target cannot
#                    link without them, and aws-lc-sys (pulled in by
#                    librqbit's rust-tls feature) compiles C with cl.exe.
#   CMake            aws-lc-sys falls back to a CMake build on some targets.
#   NASM             aws-lc-sys assembles its x86_64 crypto with NASM on
#                    Windows. Without it the build stops in aws-lc-sys
#                    before a line of Rust compiles.

$ErrorActionPreference = 'Stop'

$done = New-Object System.Collections.Generic.List[string]

function Write-Step($message) {
    Write-Host "==> $message"
    $done.Add($message) | Out-Null
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Error "winget is not available. Install 'App Installer' from the Microsoft Store, then run this again."
}

# `winget list --id X --exact` exits non-zero when the package is absent.
# Output is discarded; only the exit code is read.
# `--source winget` on every call: without it winget also queries the
# Microsoft Store source, which on a fresh Windows 11 install failed with
# 0x8a15005e "The server certificate did not match any of the expected
# values" and turned every install into "Search failed". Nothing here comes
# from the Store.
function Test-WingetPackage($id) {
    winget list --id $id --exact --source winget --accept-source-agreements *> $null
    return ($LASTEXITCODE -eq 0)
}

function Install-WingetPackage($id, $label, [string[]]$extraArgs = @()) {
    if (Test-WingetPackage $id) {
        Write-Step "$label already installed ($id)"
        return
    }
    Write-Host "Installing $label ($id)..."
    $wingetArgs = @('install', '--id', $id, '--exact', '--silent', '--source', 'winget',
        '--accept-package-agreements', '--accept-source-agreements') + $extraArgs
    & winget @wingetArgs
    # winget returns non-zero for "already installed, no upgrade available"
    # on some versions, so a failure is re-checked rather than trusted.
    if ($LASTEXITCODE -ne 0 -and -not (Test-WingetPackage $id)) {
        Write-Error "winget failed to install $id (exit code $LASTEXITCODE)."
    }
    Write-Step "$label installed ($id)"
}

# 1. Visual Studio 2022 Build Tools with the C++ workload. Without
#    --override the installer adds only the bare shell, with no compiler
#    and no linker, and the Rust build fails at the first link.
#
#    The VCTools workload carries only the x86 and x64 compilers. On an ARM64
#    machine (a Windows 11 VM on Apple Silicon) rustup picks
#    aarch64-pc-windows-msvc, and the build failed with "linker `link.exe`
#    not found" because no ARM64 linker was installed. Clang is added there
#    too, for aws-lc-sys's ARM64 build.
$isArm64 = $env:PROCESSOR_ARCHITECTURE -eq 'ARM64'
$vsComponents = '--add Microsoft.VisualStudio.Workload.VCTools'
if ($isArm64) {
    $vsComponents += ' --add Microsoft.VisualStudio.Component.VC.Tools.ARM64' +
        ' --add Microsoft.VisualStudio.Component.VC.Llvm.Clang' +
        ' --add Microsoft.VisualStudio.Component.VC.Llvm.ClangToolset'
}
Install-WingetPackage 'Microsoft.VisualStudio.2022.BuildTools' 'Visual Studio 2022 Build Tools (C++ workload)' @(
    '--override', "--wait --quiet --norestart $vsComponents --includeRecommended"
)

# winget skips a Build Tools that is already installed, --override and all, so
# an earlier install without the ARM64 compilers stayed without them. Ask
# vswhere, and modify the existing install when the component is missing.
if ($isArm64) {
    $installer = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer"
    $vswhere = Join-Path $installer 'vswhere.exe'
    $hasArm64 = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
    if (-not $hasArm64) {
        $buildTools = & $vswhere -products * -property installationPath | Select-Object -First 1
        if ($buildTools) {
            Write-Host "Adding the ARM64 compilers to $buildTools..."
            Start-Process -Wait -Verb RunAs (Join-Path $installer 'setup.exe') -ArgumentList "modify --installPath `"$buildTools`" $vsComponents --includeRecommended --quiet --wait --norestart"
        }
        $hasArm64 = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
        if (-not $hasArm64) { Write-Error "The ARM64 MSVC compilers are still missing after installing Build Tools." }
    }
    Write-Step "ARM64 MSVC compilers present"
}

# 2. CMake and NASM.
Install-WingetPackage 'Kitware.CMake' 'CMake'
Install-WingetPackage 'NASM.NASM' 'NASM'

# The NASM installer does not put itself on PATH, and aws-lc-sys looks for
# `nasm` on PATH only. Added to the *user* PATH, which needs no admin.
$nasmDirs = @(
    (Join-Path $env:ProgramFiles 'NASM'),
    (Join-Path $env:LOCALAPPDATA 'bin\NASM')
) | Where-Object { Test-Path (Join-Path $_ 'nasm.exe') }
if ($nasmDirs) {
    $nasmDir = $nasmDirs[0]
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }
    if (($userPath -split ';') -notcontains $nasmDir) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $nasmDir).TrimStart(';'), 'User')
        Write-Step "added $nasmDir to the user PATH"
    } else {
        Write-Step "NASM already on the user PATH ($nasmDir)"
    }
    if (($env:Path -split ';') -notcontains $nasmDir) { $env:Path += ";$nasmDir" }
} else {
    Write-Warning "nasm.exe not found under Program Files or LocalAppData; add its folder to PATH by hand."
}

# 3. rustup, then the MSVC stable toolchain as the default.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$rustup = Join-Path $cargoBin 'rustup.exe'
if (Test-Path $rustup) {
    Write-Step "rustup already installed ($rustup)"
} else {
    Install-WingetPackage 'Rustlang.Rustup' 'rustup'
}
# A shell opened before the install does not see ~/.cargo/bin yet.
if (($env:Path -split ';') -notcontains $cargoBin) { $env:Path += ";$cargoBin" }
if (-not (Test-Path $rustup)) {
    Write-Error "rustup.exe is not at $rustup after installing. Open a new PowerShell and run this again."
}

& $rustup default stable-msvc
if ($LASTEXITCODE -ne 0) { Write-Error "rustup default stable-msvc failed." }
Write-Step "rustup default is stable-msvc"

Write-Host ""
Write-Host "Done. What happened:"
foreach ($line in $done) { Write-Host "  - $line" }
Write-Host ""
Write-Host "Open a NEW PowerShell (so PATH changes apply), then:"
Write-Host "  cd server"
Write-Host "  cargo build --release"
