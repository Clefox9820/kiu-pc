# Reads what the scripts need from a decrypted 3DS cartridge dump: the
# headers only, never game code or data, except Read-RomCode, which only
# find-usa-addresses.ps1 uses and which keeps the bytes in memory.

Set-StrictMode -Version 2.0

# the releases this port was made and tested with
$KiuEuropeId = "0004000000030200"
$KiuUsaId = "0004000000030100"

# The NCSD header, the first partition's NCCH header and its exheader.
# Throws when the file is missing or is not a cartridge dump.
function Read-RomHeader([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "there is no file at $Path" }
    $full = (Resolve-Path -LiteralPath $Path).Path
    $stream = [System.IO.File]::OpenRead($full)
    try {
        $reader = New-Object System.IO.BinaryReader $stream
        $ncsd = $reader.ReadBytes(0x200)
        if ($ncsd.Length -lt 0x200 -or [System.Text.Encoding]::ASCII.GetString($ncsd, 0x100, 4) -ne "NCSD") {
            throw "this is not a 3DS cartridge dump (.3ds/.cci)"
        }
        $ncch = [int64][BitConverter]::ToUInt32($ncsd, 0x120) * 0x200
        $stream.Position = $ncch
        $header = $reader.ReadBytes(0x600) # NCCH header, then the exheader
    } finally { $stream.Close() }
    if ($header.Length -lt 0x600) { throw "this is not a 3DS cartridge dump (.3ds/.cci)" }
    $programId = [BitConverter]::ToUInt64($header, 0x118).ToString("X16")
    $region = switch ($programId) { $KiuEuropeId { "Europe" } $KiuUsaId { "USA" } default { $null } }
    [pscustomobject]@{
        Path = $full
        Ncch = $ncch
        Header = $header
        ProgramId = $programId
        Region = $region
        # NCCH flags byte 7, bit 2: no encryption. The exheader read above
        # is only meaningful when it is set.
        Encrypted = ($header[0x18F] -band 4) -eq 0
    }
}

# Throws unless the dump is Kid Icarus Uprising, decrypted; with -Region,
# that release only.
function Assert-KiuRom($Rom, [string]$Region) {
    if ($Region) {
        $want = if ($Region -eq "USA") { $KiuUsaId } else { $KiuEuropeId }
        if ($Rom.ProgramId -ne $want) { throw "$($Rom.Path) is $($Rom.ProgramId), not $want" }
    } elseif (-not $Rom.Region) {
        throw "this ROM is $($Rom.ProgramId), not Kid Icarus Uprising (Europe $KiuEuropeId or USA $KiuUsaId)"
    }
    if ($Rom.Encrypted) { throw "the ROM is encrypted; dump it decrypted from your own console" }
}

# The code's segments from the exheader: text, read-only data and data, with
# their address, offset in .code (each segment padded to whole pages) and size.
function Get-CodeSegment($Rom) {
    $segments = @()
    $offset = 0
    foreach ($at in 0x210, 0x220, 0x230) {
        $segments += [pscustomobject]@{
            Address = [BitConverter]::ToUInt32($Rom.Header, $at)
            File = $offset
            Size = [BitConverter]::ToUInt32($Rom.Header, $at + 8)
        }
        $offset += [BitConverter]::ToUInt32($Rom.Header, $at + 4) * 0x1000
    }
    $segments
}

# The uncompressed .code from the ExeFS, or a throw.
function Read-RomCode($Rom) {
    # ponytail: the KIU dumps seen so far keep .code uncompressed
    if ($Rom.Header[0x20D] -band 1) { throw "$($Rom.Path) has compressed code, which this script does not unpack" }
    $stream = [System.IO.File]::OpenRead($Rom.Path)
    try {
        $reader = New-Object System.IO.BinaryReader $stream
        $exefs = $Rom.Ncch + [int64][BitConverter]::ToUInt32($Rom.Header, 0x1A0) * 0x200
        $stream.Position = $exefs
        $files = $reader.ReadBytes(0x200)
        for ($k = 0; $k -lt 10; $k++) {
            if ([System.Text.Encoding]::ASCII.GetString($files, $k * 16, 6) -eq ".code`0") {
                $stream.Position = $exefs + 0x200 + [BitConverter]::ToUInt32($files, $k * 16 + 8)
                return , $reader.ReadBytes([BitConverter]::ToUInt32($files, $k * 16 + 12))
            }
        }
    } finally { $stream.Close() }
    throw "$($Rom.Path) has no .code"
}

Export-ModuleMember -Function Read-RomHeader, Assert-KiuRom, Get-CodeSegment, Read-RomCode -Variable KiuEuropeId, KiuUsaId
