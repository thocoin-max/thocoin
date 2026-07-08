# Auto-increment patch version in Cargo.toml, then sync explorer version.json.
$ErrorActionPreference = "Stop"
$cargo = "D:\Thocoin\Cargo.toml"
$verJson = "D:\thocoin-explorer\explorer\frontend\public\version.json"

$lines = Get-Content $cargo
$out = foreach ($l in $lines) {
    if ($l -match '^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') {
        $maj=[int]$matches[1]; $min=[int]$matches[2]; $pat=[int]$matches[3]+1
        $new = "$maj.$min.$pat"
        Write-Host "version -> $new"
        Set-Variable -Name NEWVER -Value $new -Scope Global
        'version = "{0}"' -f $new
    } else { $l }
}
Set-Content -Path $cargo -Value $out -Encoding UTF8

# sync version.json (no BOM)
$json = '{"version":"' + $Global:NEWVER + '","download":"https://www.thocoin.org/download"}'
[IO.File]::WriteAllText($verJson, $json, (New-Object Text.UTF8Encoding $false))
Write-Host "version.json updated to $Global:NEWVER"
