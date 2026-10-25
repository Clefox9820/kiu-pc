<#
.SYNOPSIS
Finds the USA release's address for every address the overrides name.

.DESCRIPTION
For maintainers. The overrides in overrides\ are written with the European
release's addresses, and setup.ps1 moves them to the USA release with its
$UsaAddresses table. This script reads the code of both ROMs, finds each
European function in the USA code by matching its instructions (branch
offsets are ignored, since they change when code moves), and compares the
result with that table.

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

function Fail($text) { Write-Host "error: $text" -ForegroundColor Red; exit 1 }

# the .code of a decrypted cartridge dump, as one byte per char, and its
# segments: text, read-only data and data, each padded to whole pages
function Read-Code($path, $title) {
    if (-not (Test-Path -LiteralPath $path)) { Fail "there is no file at $path" }
    $stream = [System.IO.File]::OpenRead((Resolve-Path -LiteralPath $path).Path)
    try {
        $reader = New-Object System.IO.BinaryReader $stream
        $stream.Position = 0x120
        $ncch = [int64]$reader.ReadUInt32() * 0x200
        $stream.Position = $ncch
        $header = $reader.ReadBytes(0x600) # NCCH header, then the exheader
        $id = [BitConverter]::ToUInt64($header, 0x118).ToString("X16")
        if ($id -ne $title) { Fail "$path is $id, not $title" }
        # ponytail: the KIU dumps seen so far keep .code uncompressed
        if ($header[0x20D] -band 1) { Fail "$path has compressed code, which this script does not unpack" }
        if ([BitConverter]::ToUInt32($header, 0x210) -ne 0x00100000) { Fail "$path looks encrypted" }
        $exefs = $ncch + [int64][BitConverter]::ToUInt32($header, 0x1A0) * 0x200
        $stream.Position = $exefs
        $files = $reader.ReadBytes(0x200)
        $code = $null
        for ($k = 0; $k -lt 10; $k++) {
            if ([Text.Encoding]::ASCII.GetString($files, $k * 16, 6) -eq ".code`0") {
                $stream.Position = $exefs + 0x200 + [BitConverter]::ToUInt32($files, $k * 16 + 8)
                $code = $reader.ReadBytes([BitConverter]::ToUInt32($files, $k * 16 + 12))
            }
        }
    } finally { $stream.Close() }
    if (-not $code) { Fail "$path has no .code" }
    $segments = @()
    $offset = 0
    foreach ($at in 0x210, 0x220, 0x230) {
        $segments += [pscustomobject]@{
            Address = [BitConverter]::ToUInt32($header, $at)
            File = $offset
            Size = [BitConverter]::ToUInt32($header, $at + 8)
        }
        $offset += [BitConverter]::ToUInt32($header, $at + 4) * 0x1000
    }
    [pscustomobject]@{
        Bytes = $code
        Chars = [Text.Encoding]::GetEncoding(28591).GetString($code)
        Segments = $segments
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
$eurCode = Read-Code $Europe "0004000000030200"
$usaCode = Read-Code $Usa "0004000000030100"
$text = $eurCode.Segments[0]

# the addresses the overrides name in the game's code, comments included,
# since setup.ps1 moves those too; Zakuro's mailbox and constants fall outside
$addresses = @()
foreach ($file in Get-ChildItem -Path (Join-Path $Root "overrides") -Filter *.c) {
    foreach ($m in [regex]::Matches((Get-Content -Raw $file.FullName), '0x[0-9A-Fa-f]{8}')) {
        $value = [Convert]::ToUInt32($m.Value.Substring(2), 16)
        if ($value -ge $text.Address -and $value -lt $text.Address + $text.Size) { $addresses += $value }
    }
}
$addresses = $addresses | Sort-Object -Unique

$table = @{}
$setup = Get-Content -Raw (Join-Path $PSScriptRoot "setup.ps1")
foreach ($m in [regex]::Matches($setup, '"(0x[0-9A-Fa-f]{8})" = "(0x[0-9A-Fa-f]{8})"')) {
    $table[[Convert]::ToUInt32($m.Groups[1].Value.Substring(2), 16)] = [Convert]::ToUInt32($m.Groups[2].Value.Substring(2), 16)
}

$problems = 0
foreach ($address in $addresses) {
    $found = Find-Usa $eurCode $usaCode $address
    $line = '    "0x{0:X8}" = ' -f $address
    if ($null -eq $found) {
        $problems++
        Write-Host ($line + '?           not found, find it by hand') -ForegroundColor Red
    } elseif (-not $table.ContainsKey($address)) {
        $problems++
        Write-Host ($line + '"0x{0:X8}"  missing from setup.ps1' -f $found) -ForegroundColor Yellow
    } elseif ($table[$address] -ne $found) {
        $problems++
        Write-Host ($line + '"0x{0:X8}"  setup.ps1 has 0x{1:X8}' -f $found, $table[$address]) -ForegroundColor Yellow
    } else {
        Write-Host ($line + '"0x{0:X8}"  ok' -f $found)
    }
}
if ($problems) {
    Write-Host "`n$problems to fix in `$UsaAddresses in scripts\setup.ps1" -ForegroundColor Yellow
    exit 1
}
Write-Host "`nsetup.ps1 has every USA address"
