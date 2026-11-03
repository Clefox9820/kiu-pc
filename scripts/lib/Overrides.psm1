# The overrides in overrides\ are written with the European release's
# addresses. These move them to the USA release with the table in
# overrides\usa-addresses.psd1, which scripts\find-usa-addresses.ps1 checks
# against both ROMs.

Set-StrictMode -Version 2.0

$AddressPattern = '0x[0-9A-Fa-f]{8}'

# The table as European address => USA address, both [uint32].
function Get-UsaAddressTable([string]$Path) {
    $table = @{}
    $data = Import-PowerShellDataFile -LiteralPath $Path
    foreach ($key in $data.Keys) {
        $table[[Convert]::ToUInt32(([string]$key).Substring(2), 16)] = [Convert]::ToUInt32(([string]$data[$key]).Substring(2), 16)
    }
    $table
}

# The override sources 3dsrecomp reads from a folder: its *.c files.
function Get-OverrideFile([string]$Directory) {
    Get-ChildItem -LiteralPath $Directory -Filter *.c -File | Sort-Object Name
}

# Every 8-digit hex number in the overrides that falls in [Start, End),
# comments included, as File and Address.
function Get-OverrideAddress([string]$Directory, [uint32]$Start, [uint64]$End) {
    foreach ($file in Get-OverrideFile $Directory) {
        $source = [System.IO.File]::ReadAllText($file.FullName)
        foreach ($m in [regex]::Matches($source, $AddressPattern)) {
            $value = [Convert]::ToUInt32($m.Value.Substring(2), 16)
            if ($value -ge $Start -and $value -lt $End) {
                [pscustomobject]@{ File = $file.Name; Address = $value }
            }
        }
    }
}

# The addresses in [Start, End) the overrides name without a USA address in
# the table: what would stay European in the USA build. Empty when all map.
function Find-UnmappedAddress([string]$Directory, [hashtable]$Table, [uint32]$Start, [uint64]$End) {
    Get-OverrideAddress $Directory $Start $End | Where-Object { -not $Table.ContainsKey($_.Address) }
}

# The source with every address the table names replaced by its USA one.
function Convert-ToUsaAddress([string]$Source, [hashtable]$Table) {
    [regex]::Replace($Source, $AddressPattern, {
        param($m)
        $value = [Convert]::ToUInt32($m.Value.Substring(2), 16)
        if ($Table.ContainsKey($value)) { '0x{0:X8}' -f $Table[$value] } else { $m.Value }
    })
}

Export-ModuleMember -Function Get-UsaAddressTable, Get-OverrideFile, Get-OverrideAddress, Find-UnmappedAddress, Convert-ToUsaAddress
