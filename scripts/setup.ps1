<#
.SYNOPSIS
Builds Kid Icarus Uprising for PC from your own copy of the game.

.DESCRIPTION
Recompiles the game's code with 3dsrecomp and links it with the Zakuro in
this repository. Nothing from the game is in the repository: the ROM you
give is read here, on your machine, and what is made from it stays in
build\, which git ignores.

Needs Windows 10 or newer, about 6 GB of free disk and about 75 minutes per
release to translate, plus about 10 the first time for the tools. Close
heavy programs first: 3dsrecomp runs as many compilers at once as the free
memory allows, measured once at the start.

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

Import-Module (Join-Path $PSScriptRoot "lib\Rom.psm1") -Force
Import-Module (Join-Path $PSScriptRoot "lib\Overrides.psm1") -Force

$RecompRepo = "https://github.com/fearkov/3dsrecomp"
$RecompCommit = "c3f21f94ba50d688025febc8974ff348d471b587"
$WinLibsUrl = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
# the SHA-256 GitHub gives for that file
$WinLibsSha256 = "3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1"
$RustupUrl = "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe"
# the Rust a new install gets, the one the port was last built with, so a
# build made later does not change with whatever is stable that day
$RustVersion = "1.99.1"

$Root = Split-Path -Parent $PSScriptRoot
$Tools = Join-Path $Root "tools"
$Build = Join-Path $Root "build"
$Overrides = Join-Path $Root "overrides"
$UsaTable = Join-Path $Overrides "usa-addresses.psd1"
$Recomp = Join-Path $Tools "3dsrecomp"
$RecompExe = Join-Path $Recomp "target\release\3dsrecomp.exe"

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }

# Deletes a download whose SHA-256 is not the one expected and stops: a
# file cut short or changed on the way must never be unpacked or run.
function Assert-Sha256([string]$path, [string]$expected) {
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
    if ($actual -ne $expected.ToUpperInvariant()) {
        Remove-Item -LiteralPath $path -Force
        Fail "$(Split-Path -Leaf $path) is not the file expected (SHA-256 $actual, wanted $($expected.ToUpperInvariant())); run setup again"
    }
}

function Run($exe, [string[]]$arguments, $where) {
    Push-Location $where
    try {
        & $exe @arguments
        if ($LASTEXITCODE -ne 0) { Fail "$exe $($arguments -join ' ') failed ($LASTEXITCODE)" }
    } finally { Pop-Location }
}

# Windows PowerShell's Set-Content writes the ANSI code page, which mangles
# a path such as C:\Users\Jose with an accent; cargo and rustc read UTF-8.
function Write-Utf8NoBom([string]$path, [string]$text) {
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding $false))
}

function Read-Text([string]$path) { [System.IO.File]::ReadAllText($path) }

# --- the ROM: Kid Icarus Uprising, decrypted -------------------------------
function Test-Rom([string]$path) {
    Step "Checking the ROM"
    try {
        $info = Read-RomHeader $path
        Assert-KiuRom $info
    } catch { Fail $_.Exception.Message }
    Write-Host "Kid Icarus Uprising ($($info.Region)), decrypted: OK"
    $info
}

# --- tools, inside the repository so nothing else on the system changes ----
function Install-Gcc {
    Step "C compiler (MinGW-w64 gcc)"
    $mingw = Join-Path $Tools "mingw64"
    if (-not (Test-Path (Join-Path $mingw "bin\gcc.exe"))) {
        $zip = Join-Path $Tools "winlibs.zip"
        $unpack = Join-Path $Tools "winlibs.unpack"
        Write-Host "Downloading WinLibs gcc (about 280 MB)..."
        Invoke-WebRequest -UseBasicParsing -Uri $WinLibsUrl -OutFile $zip
        Assert-Sha256 $zip $WinLibsSha256
        Write-Host "Unpacking..."
        # into a folder of its own, renamed once whole: an interrupted unpack
        # must not leave a mingw64 that a later run takes for a complete one
        foreach ($stale in $unpack, $mingw) {
            if (Test-Path $stale) { Remove-Item -Recurse -Force $stale }
        }
        Expand-Archive -LiteralPath $zip -DestinationPath $unpack -Force
        Move-Item -LiteralPath (Join-Path $unpack "mingw64") -Destination $mingw
        Remove-Item -Recurse -Force $unpack
        Remove-Item $zip
    }
    & gcc --version | Select-Object -First 1
}

function Install-Rust {
    Step "Rust (GNU toolchain)"
    if (-not (Test-Path (Join-Path $Tools "cargo\bin\cargo.exe"))) {
        $installer = Join-Path $Tools "rustup-init.exe"
        Invoke-WebRequest -UseBasicParsing -Uri $RustupUrl -OutFile $installer
        # rustup publishes each installer's SHA-256 next to it
        $published = (Invoke-WebRequest -UseBasicParsing -Uri "$RustupUrl.sha256").Content
        if ($published -is [byte[]]) { $published = [System.Text.Encoding]::ASCII.GetString($published) }
        Assert-Sha256 $installer ($published.Trim() -split '\s+')[0]
        Run $installer @("-y", "--no-modify-path", "--default-host", "x86_64-pc-windows-gnu", "--default-toolchain", $RustVersion, "--profile", "minimal") $Tools
        Remove-Item $installer
    }
    # every cargo run of this setup uses the install's default toolchain:
    # 3dsrecomp and zakuro carry a rust-toolchain.toml asking for stable,
    # which would otherwise download whatever stable is that day. a new
    # install's default is $RustVersion; an older one keeps its own.
    $default = (& rustup default) -split '\s+' | Select-Object -First 1
    if (-not $default) { Fail "rustup has no default toolchain" }
    $env:RUSTUP_TOOLCHAIN = $default
    & cargo --version
}

# --- 3dsrecomp, at the version the port was made with -----------------------
function Install-Recomp {
    Step "3dsrecomp"
    if (-not (Test-Path (Join-Path $Recomp ".git"))) {
        Run git @("clone", "--quiet", $RecompRepo, $Recomp) $Tools
    }
    Run git @("checkout", "--quiet", $RecompCommit) $Recomp
    Run cargo @("build", "--release", "--quiet") $Recomp
}

# --- the overrides, moved to the USA release's addresses --------------------
# A copy in build\overrides-usa. Stops when an override names an address in
# the game's code that the table does not move: it would stay European.
function Copy-UsaOverride($info) {
    $table = Get-UsaAddressTable $UsaTable
    $text = @(Get-CodeSegment $info)[0]
    $missing = @(Find-UnmappedAddress $Overrides $table $text.Address ([uint64]$text.Address + $text.Size))
    if ($missing.Count -gt 0) {
        foreach ($m in $missing) {
            Write-Host ("error: {0} names 0x{1:X8}, which has no USA address in overrides\usa-addresses.psd1 yet" -f $m.File, $m.Address) -ForegroundColor Red
        }
        Fail "find them with scripts\find-usa-addresses.ps1"
    }
    $moved = Join-Path $Build "overrides-usa"
    New-Item -ItemType Directory -Force -Path $moved | Out-Null
    Get-ChildItem -LiteralPath $moved -Filter *.c | Remove-Item
    foreach ($file in Get-OverrideFile $Overrides) {
        $source = Convert-ToUsaAddress (Read-Text $file.FullName) $table
        Write-Utf8NoBom (Join-Path $moved $file.Name) $source
    }
    $moved
}

# --- the game's code, translated from your ROM (75 minutes the first time) --
function Invoke-Translation($info, [string]$port) {
    Step "Translating the game's code (the first time takes about 75 minutes)"
    if ($Force -and (Test-Path $port)) { Remove-Item -Recurse -Force $port }
    if (Test-Path (Join-Path $port "code\librecomp.a")) {
        Write-Host "already translated, in $port (-Force translates again)"
        return
    }
    $from = if ($info.Region -eq "USA") { Copy-UsaOverride $info } else { $Overrides }
    # 3dsrecomp archives the ~810 objects with their whole paths when the
    # overrides' objects sit in a folder of their own, and from a deep enough
    # folder that command line passes Windows' 32767 characters. A drive
    # letter of its own for build\ keeps the paths short.
    $letter = [char[]]"ZYXWVUTSRQPONMLK" | Where-Object { -not (Test-Path "${_}:\") } | Select-Object -First 1
    if (-not $letter) { Fail "no free drive letter to shorten build\'s paths with" }
    & subst "${letter}:" $Build
    if ($LASTEXITCODE -ne 0) { Fail "subst ${letter}: $Build failed ($LASTEXITCODE)" }
    try {
        Run $RecompExe @("port", $info.Path, "${letter}:\$(Split-Path -Leaf $port)", "--name", "kid-icarus", "--overrides", $from) "${letter}:\"
    } finally {
        & subst "${letter}:" /D | Out-Null
    }
}

# --- the program, against this repository's Zakuro --------------------------
function Build-Program([string]$port) {
    Step "Building the program"
    $manifest = Join-Path $port "Cargo.toml"
    $zakuro = (Join-Path $Root "zakuro\crates\zakuro") -replace '\\', '/'
    $text = Read-Text $manifest
    $text = $text -replace 'zakuro = \{[^}]*\}', "zakuro = { path = `"$zakuro`" }"
    if ($text -notmatch '\[profile\.release\]') {
        # what Zakuro builds itself with, which a dependency doesn't pass on
        $text += "`n[profile.release]`nopt-level = 3`nlto = `"thin`"`ncodegen-units = 4`npanic = `"unwind`"`n"
    }
    Write-Utf8NoBom $manifest $text
    New-Item -ItemType Directory -Force -Path (Join-Path $port ".cargo") | Out-Null
    # built for this machine's processor only
    Write-Utf8NoBom (Join-Path $port ".cargo\config.toml") "[build]`nrustflags = [`"-C`", `"target-cpu=native`"]`r`n"
    # the crate versions Zakuro is tested with, and 3dsrecomp's runtime at the
    # commit that translated the code; left alone, cargo takes 3dsrecomp's main
    # of the day, which the translated code was not made for
    Copy-Item -Force (Join-Path $Root "zakuro\Cargo.lock") (Join-Path $port "Cargo.lock")
    Run cargo @("update", "--quiet", "-p", "recomp3ds", "-p", "recomp-abi", "--precise", $RecompCommit) $port
    Run cargo @("build", "--release") $port
}

# --- the program's icon and name, the icon taken from your ROM ---------------
function Set-ProgramIcon([string]$port, [string]$exe, [string]$rom) {
    Step "Giving the program the game's icon and name"
    $resource = Join-Path $port "program.res"
    $buildScript = Join-Path $port "build.rs"
    # done only once the build script links what windres made
    $done = (Test-Path $resource) -and (Read-Text $buildScript).Contains("program.res")
    if (-not $Force -and $done) {
        Write-Host "already done (-Force does it again)"
        return
    }
    Run $exe @("--save-icon", (Join-Path $port "icon.ico"), $rom) $port
    # an unofficial build, so it names no company
    Write-Utf8NoBom (Join-Path $port "program.rc") (@'
1 ICON "icon.ico"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Kid Icarus: Uprising"
      VALUE "ProductName", "Kid Icarus: Uprising for PC"
      VALUE "Comments", "Unofficial, built from your own copy of the game"
      VALUE "InternalName", "kid-icarus"
      VALUE "OriginalFilename", "kid-icarus.exe"
      VALUE "FileVersion", "0.1.0"
      VALUE "ProductVersion", "0.1.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
'@ + "`r`n")
    Run windres @("program.rc", "-O", "coff", "-o", "program.res") $port
    # the build script 3dsrecomp wrote links it into the program
    $text = Read-Text $buildScript
    if (-not $text.Contains("program.res")) {
        $link = 'println!("cargo:rerun-if-changed=program.res"); println!("cargo:rustc-link-arg-bins={}/program.res", var("CARGO_MANIFEST_DIR").unwrap());'
        Write-Utf8NoBom $buildScript $text.Replace("fn main() {", "fn main() {`n    $link")
    }
    Run cargo @("build", "--release") $port
}

# --- Play.bat, in the repository root ----------------------------------------
# cmd reads a batch file in the console's code page: 65001 makes it read this
# one as the UTF-8 it is written in. The program is found next to the batch
# file; % in the ROM's path is doubled so cmd does not take it for a variable.
function Write-PlayBat([string]$exe, [string]$rom) {
    $play = Join-Path $Root "Play.bat"
    $program = $exe.Substring($Root.Length).TrimStart('\')
    $romArg = $rom -replace '%', '%%'
    Write-Utf8NoBom $play "@echo off`r`nchcp 65001>nul`r`n`"%~dp0$program`" `"$romArg`"`r`n"
    $play
}

# --- main ---------------------------------------------------------------------
$info = Test-Rom $Rom
if ($CheckOnly) { exit 0 }
# each release in its own folder, so building one keeps the other
$Port = Join-Path $Build $(if ($info.Region -eq "USA") { "port-usa" } else { "port" })
$Exe = Join-Path $Port "target\release\kid-icarus.exe"

New-Item -ItemType Directory -Force -Path $Tools, $Build | Out-Null
. (Join-Path $PSScriptRoot "env.ps1")
Install-Gcc
Install-Rust
Install-Recomp
Invoke-Translation $info $Port
Build-Program $Port
Set-ProgramIcon $Port $Exe $info.Path
$Play = Write-PlayBat $Exe $info.Path
Step "Done"
Write-Host "Play with $Play"
