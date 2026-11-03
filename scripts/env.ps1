# Points this PowerShell session at the toolchain scripts\setup.ps1 installs
# in tools\: Rust (GNU target) and WinLibs gcc. Dot-source it from the
# repository root:
#   . scripts\env.ps1
# It changes only this session's environment, nothing on the system.

$KiuTools = Join-Path (Split-Path -Parent $PSScriptRoot) "tools"
$env:RUSTUP_HOME = Join-Path $KiuTools "rustup"
$env:CARGO_HOME = Join-Path $KiuTools "cargo"
foreach ($KiuBin in (Join-Path $KiuTools "mingw64\bin"), (Join-Path $KiuTools "cargo\bin")) {
    if (($env:PATH -split ';') -notcontains $KiuBin) { $env:PATH = "$KiuBin;$env:PATH" }
}
$env:CC = "gcc"
$env:AR = "ar"
