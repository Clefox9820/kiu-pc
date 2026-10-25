<#
.SYNOPSIS
Builds Kid Icarus Uprising for PC from your own copy of the game.

.DESCRIPTION
Recompiles the game's code with 3dsrecomp and links it with the Zakuro in
this repository. Nothing from the game is in the repository: the ROM you
give is read here, on your machine, and what is made from it stays in
build\, which git ignores.

Needs Windows 10 or newer, about 6 GB of free disk and, for the first build,
around an hour. Close heavy programs first: 3dsrecomp runs as many
compilers at once as the free memory allows, measured once at the start.

.PARAMETER Rom
Your decrypted dump of the game, a .3ds or .cci file.

.PARAMETER CheckOnly
Only checks that the ROM is the right game and decrypted, then stops.

.PARAMETER Force
Translates the game again even when build\port is already there.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 -Rom "D:\dumps\kid icarus.cci"
#>
param(
    [Parameter(Mandatory = $true)][string]$Rom,
    [switch]$Force,
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# the versions this port was made and tested with
$Title = "0004000000030200"
$RecompRepo = "https://github.com/fearkov/3dsrecomp"
$RecompCommit = "c3f21f94ba50d688025febc8974ff348d471b587"
$WinLibsUrl = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
$RustupUrl = "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe"

$Root = Split-Path -Parent $PSScriptRoot
$Tools = Join-Path $Root "tools"
$Build = Join-Path $Root "build"
$Port = Join-Path $Build "port"

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }

function Run($exe, [string[]]$arguments, $where) {
    Push-Location $where
    try {
        & $exe @arguments
        if ($LASTEXITCODE -ne 0) { Fail "$exe $($arguments -join ' ') failed ($LASTEXITCODE)" }
    } finally { Pop-Location }
}

# --- the ROM: Kid Icarus Uprising, decrypted -------------------------------
Step "Checking the ROM"
if (-not (Test-Path -LiteralPath $Rom)) { Fail "there is no file at $Rom" }
$Rom = (Resolve-Path -LiteralPath $Rom).Path
$stream = [System.IO.File]::OpenRead($Rom)
try {
    $header = New-Object byte[] 0x200
    [void]$stream.Read($header, 0, 0x200)
    if ([System.Text.Encoding]::ASCII.GetString($header, 0x100, 4) -ne "NCSD") {
        Fail "this is not a 3DS cartridge dump (.3ds/.cci)"
    }
    $offset = [BitConverter]::ToUInt32($header, 0x120) * 0x200
    [void]$stream.Seek($offset, "Begin")
    $ncch = New-Object byte[] 0x200
    [void]$stream.Read($ncch, 0, 0x200)
} finally { $stream.Close() }
$programId = ([BitConverter]::ToUInt64($ncch, 0x118)).ToString("X16")
if ($programId -ne $Title) { Fail "this ROM is $programId, not Kid Icarus Uprising ($Title)" }
# NCCH flags byte 7, bit 2: no encryption
if (($ncch[0x18F] -band 4) -eq 0) { Fail "the ROM is encrypted; dump it decrypted from your own console" }
Write-Host "Kid Icarus Uprising, decrypted: OK"
if ($CheckOnly) { exit 0 }

# --- tools, inside the repository so nothing else on the system changes ----
New-Item -ItemType Directory -Force -Path $Tools, $Build | Out-Null
$env:RUSTUP_HOME = Join-Path $Tools "rustup"
$env:CARGO_HOME = Join-Path $Tools "cargo"
$env:PATH = "$(Join-Path $Tools 'cargo\bin');$(Join-Path $Tools 'mingw64\bin');$env:PATH"
$env:CC = "gcc"
$env:AR = "ar"

Step "C compiler (MinGW-w64 gcc)"
if (-not (Test-Path (Join-Path $Tools "mingw64\bin\gcc.exe"))) {
    $zip = Join-Path $Tools "winlibs.zip"
    Write-Host "Downloading WinLibs gcc (about 280 MB)..."
    Invoke-WebRequest -UseBasicParsing -Uri $WinLibsUrl -OutFile $zip
    Write-Host "Unpacking..."
    Expand-Archive -LiteralPath $zip -DestinationPath $Tools -Force
    Remove-Item $zip
}
& gcc --version | Select-Object -First 1

Step "Rust (GNU toolchain)"
if (-not (Test-Path (Join-Path $Tools "cargo\bin\cargo.exe"))) {
    $installer = Join-Path $Tools "rustup-init.exe"
    Invoke-WebRequest -UseBasicParsing -Uri $RustupUrl -OutFile $installer
    Run $installer @("-y", "--no-modify-path", "--default-host", "x86_64-pc-windows-gnu", "--default-toolchain", "stable", "--profile", "minimal") $Tools
    Remove-Item $installer
}
& cargo --version

# --- 3dsrecomp, at the version the port was made with -----------------------
Step "3dsrecomp"
$Recomp = Join-Path $Tools "3dsrecomp"
if (-not (Test-Path (Join-Path $Recomp ".git"))) {
    Run git @("clone", "--quiet", $RecompRepo, $Recomp) $Tools
}
Run git @("checkout", "--quiet", $RecompCommit) $Recomp
Run cargo @("build", "--release", "--quiet") $Recomp
$RecompExe = Join-Path $Recomp "target\release\3dsrecomp.exe"

# --- the game's code, translated from your ROM (an hour the first time) -----
Step "Translating the game's code (the first time takes about an hour)"
if ($Force -and (Test-Path $Port)) { Remove-Item -Recurse -Force $Port }
if (-not (Test-Path (Join-Path $Port "code\librecomp.a"))) {
    $Overrides = Join-Path $Root "overrides"
    Run $RecompExe @("port", $Rom, $Port, "--name", "kid-icarus", "--overrides", $Overrides) $Build
} else {
    Write-Host "already translated, in build\port (-Force translates again)"
}

# --- the program, against this repository's Zakuro --------------------------
Step "Building the program"
$manifest = Join-Path $Port "Cargo.toml"
$zakuro = (Join-Path $Root "zakuro\crates\zakuro") -replace '\\', '/'
$text = Get-Content -Raw $manifest
$text = $text -replace 'zakuro = \{[^}]*\}', "zakuro = { path = `"$zakuro`" }"
if ($text -notmatch '\[profile\.release\]') {
    # what Zakuro builds itself with, which a dependency doesn't pass on
    $text += "`n[profile.release]`nopt-level = 3`nlto = `"thin`"`ncodegen-units = 4`npanic = `"unwind`"`n"
}
Set-Content -NoNewline -Path $manifest -Value $text
New-Item -ItemType Directory -Force -Path (Join-Path $Port ".cargo") | Out-Null
# built for this machine's processor only
Set-Content -Path (Join-Path $Port ".cargo\config.toml") -Value "[build]`nrustflags = [`"-C`", `"target-cpu=native`"]"
Run cargo @("build", "--release") $Port
$Exe = Join-Path $Port "target\release\kid-icarus.exe"

$Play = Join-Path $Root "Play.bat"
Set-Content -Path $Play -Value "@echo off`r`n`"$Exe`" `"$Rom`""
Step "Done"
Write-Host "Play with $Play"
