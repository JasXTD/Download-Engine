[CmdletBinding()]
param([switch]$Signed)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Release-Version.ps1')
. (Join-Path $PSScriptRoot 'Release-Files.ps1')
$version = Get-ReleaseVersion -Root $workspaceRoot
$files = @(Get-ReleaseSourceFiles -Root $workspaceRoot)
Assert-PublicReleaseContents -Root $workspaceRoot -Version $version -Signed:$Signed
$outputDirectory = Get-PublicReleaseDirectory -Root $workspaceRoot -Version $version
& (Join-Path $PSScriptRoot 'Prepare-MediaTools.ps1')
& node (Join-Path $PSScriptRoot 'Build-LicenseBundle.cjs') --check
if ($LASTEXITCODE -ne 0) { throw 'Dependency source/license checks failed.' }
$stageRoot = Join-Path $workspaceRoot 'artifacts'
$stage = Join-Path $stageRoot ('release-source-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage -Force | Out-Null
try {
    foreach ($relative in $files) {
        $source = Resolve-ReleasePath -Root $workspaceRoot -Relative $relative
        $destination = Join-Path $stage $relative
        New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination
    }
    # Validate the bytes actually archived, including the hash-pinned exceptions.
    $null = Get-ReleaseSourceFiles -Root $stage
    $output = Join-Path $outputDirectory "DownloadEngine-$version-Source.zip"
    $temporaryArchive = "$stage.zip"
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($stage, $temporaryArchive, [IO.Compression.CompressionLevel]::Optimal, $false)
    Assert-PublicReleaseContents -Root $workspaceRoot -Version $version -Signed:$Signed
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    Move-Item -LiteralPath $temporaryArchive -Destination $output -Force
    Get-FileHash -LiteralPath $output -Algorithm SHA256
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if ((Split-Path $resolvedStage -Parent) -eq [IO.Path]::GetFullPath($stageRoot) -and (Split-Path $resolvedStage -Leaf) -match '^release-source-[0-9a-f]{32}$') {
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
        if (Test-Path -LiteralPath "$resolvedStage.zip") { Remove-Item -LiteralPath "$resolvedStage.zip" -Force }
    }
    else { throw 'Unexpected source staging path; refusing cleanup.' }
}
