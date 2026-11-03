<#
.SYNOPSIS
Finds the USA release's address for every address the overrides name.

.DESCRIPTION
For maintainers. The overrides in overrides\ are written with the European
release's addresses, and setup.ps1 moves them to the USA release with the
table in overrides\usa-addresses.psd1. This script reads the code of both
ROMs, finds each European function in the USA code by matching its
instructions (branch offsets are ignored, since they change when code
moves), and compares the result with that table.

It prints only addresses, never game bytes. It exits with 1 when an address
is missing from the table, differs from it, or cannot be found.

.PARAMETER Europe
A decrypted dump of the European release (0004000000030200).

.PARAMETER Usa
A decrypted dump of the USA release (0004000000030100).

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\find-usa-addresses.ps1 -Europe "D:\dumps\kiu-eur.cci" -Usa "D:\dumps\kiu-usa.3ds"
#>
param(
    [Parameter(Mandatory = $true)][string]$Europe,
    [Parameter(Mandatory = $true)][string]$Usa
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Overrides = Join-Path $Root "overrides"

Import-Module (Join-Path $PSScriptRoot "lib\Rom.psm1") -Force
Import-Module (Join-Path $PSScriptRoot "lib\Overrides.psm1") -Force

function Fail($text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }

# the .code of a decrypted cartridge dump of that release, as bytes and as
# one byte per char, and its segments
function Read-Code($path, $region) {
    try {
        $rom = Read-RomHeader $path
        Assert-KiuRom $rom $region
        $code = Read-RomCode $rom
    } catch { Fail $_.Exception.Message }
    [pscustomobject]@{
        Bytes = $code
        Chars = [Text.Encoding]::GetEncoding(28591).GetString($code)
        Segments = @(Get-CodeSegment $rom)
    }
}

function To-File($rom, [uint32]$address) {
    foreach ($s in $rom.Segments) {
        if ($address -ge $s.Address -and $address -lt $s.Address + $s.Size) { return $address - $s.Address + $s.File }
    }
    return $null
}

function To-Address($rom, $file) {
    foreach ($s in $rom.Segments) {
        if ($file -ge $s.File -and $file -lt $s.File + $s.Size) { return [uint32]($file - $s.File + $s.Address) }
    }
    return $null
}

# every aligned start in chars matching the instructions at file in rom
function Find-Like($rom, $file, $words, $thumb, $chars) {
    $pattern = New-Object System.Text.StringBuilder
    for ($k = 0; $k -lt $words; $k++) {
        $at = $file + 4 * $k
        $word = [BitConverter]::ToUInt32($rom.Bytes, $at)
        # an ARM B or BL: its offset changes when code moves, keep only the opcode
        $branch = -not $thumb -and (($word -shr 25) -band 7) -eq 5
        for ($b = 0; $b -lt 4; $b++) {
            if ($branch -and $b -lt 3) { [void]$pattern.Append("[\s\S]") }
            else { [void]$pattern.Append(("\x{0:X2}" -f $rom.Bytes[$at + $b])) }
        }
    }
    $regex = New-Object System.Text.RegularExpressions.Regex ("(?=" + $pattern + ")")
    @($regex.Matches($chars) | Where-Object { $_.Index % 2 -eq 0 -and ($thumb -or $_.Index % 4 -eq 0) } | ForEach-Object { $_.Index })
}

# the USA address of a European one, or null when no window matches once in each
function Find-Usa($eur, $usa, [uint32]$address) {
    $thumb = ($address -band 1) -eq 1
    $file = To-File $eur ($address -band 0xFFFFFFFE)
    # from the address, then windows before it, for call sites inside a function
    foreach ($window in @(@(0, 16), @(-64, 16), @(-32, 8), @(0, 8))) {
        $start = $file + $window[0]
        if ($start -lt 0 -or $start + 4 * $window[1] -gt $eur.Bytes.Length) { continue }
        if (@(Find-Like $eur $start $window[1] $thumb $eur.Chars).Count -ne 1) { continue }
        $hits = @(Find-Like $eur $start $window[1] $thumb $usa.Chars)
        if ($hits.Count -eq 1) {
            $found = To-Address $usa ($hits[0] - $window[0])
            if ($null -ne $found) { return [uint32]($found -bor ($address -band 1)) }
        }
    }
    return $null
}

# not $usa: PowerShell names ignore case, so it would be the -Usa string
$eurCode = Read-Code $Europe "Europe"
$usaCode = Read-Code $Usa "USA"
$text = $eurCode.Segments[0]

# the addresses the overrides name in the game's code, comments included,
# since setup.ps1 moves those too and stops on any it cannot move; Zakuro's
# mailbox and constants fall outside
$addresses = @(Get-OverrideAddress $Overrides $text.Address ([uint64]$text.Address + $text.Size) | ForEach-Object { $_.Address } | Sort-Object -Unique)
$table = Get-UsaAddressTable (Join-Path $Overrides "usa-addresses.psd1")

$problems = 0
foreach ($address in $addresses) {
    $found = Find-Usa $eurCode $usaCode $address
    $line = '    "0x{0:X8}" = ' -f $address
    if ($null -eq $found) {
        $problems++
        Write-Host ($line + '?           not found, find it by hand') -ForegroundColor Red
    } elseif (-not $table.ContainsKey($address)) {
        $problems++
        Write-Host ($line + '"0x{0:X8}"  missing from the table' -f $found) -ForegroundColor Yellow
    } elseif ($table[$address] -ne $found) {
        $problems++
        Write-Host ($line + '"0x{0:X8}"  the table has 0x{1:X8}' -f $found, $table[$address]) -ForegroundColor Yellow
    } else {
        Write-Host ($line + '"0x{0:X8}"  ok' -f $found)
    }
}
if ($problems) {
    Write-Host "`n$problems to fix in overrides\usa-addresses.psd1" -ForegroundColor Yellow
    exit 1
}
Write-Host "`noverrides\usa-addresses.psd1 has every USA address"
