[CmdletBinding()]
param([switch]$Signed)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Release-Version.ps1')
. (Join-Path $PSScriptRoot 'Release-Files.ps1')
$version = Get-ReleaseVersion -Root $workspaceRoot
Assert-PublicReleaseContents -Root $workspaceRoot -Version $version -Signed:$Signed -RequireComplete
$directory = Get-PublicReleaseDirectory -Root $workspaceRoot -Version $version
$names = @(Get-PublicReleaseFileNames -Version $version -Signed:$Signed)
$lines = @(foreach ($name in $names) {
    $file = Join-Path $directory $name
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing public release artifact: $name" }
    '{0}  {1}' -f (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant(), $name
})
$output = Join-Path $directory "SHA256SUMS-$version"
[IO.File]::WriteAllText($output, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
Write-Output "Wrote checksums for $($names.Count) public release assets to $output"
