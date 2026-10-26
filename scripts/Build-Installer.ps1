[CmdletBinding()]
param(
    [string]$Compiler,
    [switch]$SkipBuild,
    [switch]$TestBuild,
    [string]$SignTool,
    [string]$CertificateThumbprint,
    [string]$TimestampUrl,
    [switch]$AllowUnsignedDevelopment,
    [switch]$AllowUnsignedRelease
)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Signing.ps1')
. (Join-Path $PSScriptRoot 'Release-Version.ps1')
. (Join-Path $PSScriptRoot 'Release-Files.ps1')
$version = Get-ReleaseVersion -Root $workspaceRoot
$signingParameters = @{
    SignTool = $SignTool
    CertificateThumbprint = $CertificateThumbprint
    TimestampUrl = $TimestampUrl
    AllowUnsignedDevelopment = $AllowUnsignedDevelopment
    AllowUnsignedRelease = $AllowUnsignedRelease
}
$signing = Resolve-CodeSigningConfiguration @signingParameters
if ($AllowUnsignedDevelopment -and -not $TestBuild) {
    throw '-AllowUnsignedDevelopment requires -TestBuild so an unsigned installer cannot use a release filename.'
}
if ($AllowUnsignedRelease -and $TestBuild) {
    throw '-AllowUnsignedRelease cannot be combined with -TestBuild. Use -AllowUnsignedDevelopment for an isolated test installer.'
}
$null = Get-ReleaseSourceFiles -Root $workspaceRoot
$outputDirectory = Join-Path $workspaceRoot 'dist'
if (-not $TestBuild) {
    Assert-PublicReleaseContents -Root $workspaceRoot -Version $version -Signed:([bool]$signing)
    $outputDirectory = Get-PublicReleaseDirectory -Root $workspaceRoot -Version $version
}
if (-not $Compiler) {
    $candidates = @(
        (Join-Path $workspaceRoot 'artifacts\build-tools\InnoSetup\ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 7\ISCC.exe')
    )
    $Compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $Compiler -or -not (Test-Path -LiteralPath $Compiler -PathType Leaf)) {
    throw 'Install the official Inno Setup compiler, then pass -Compiler <absolute path to ISCC.exe>.'
}
Push-Location $workspaceRoot
try {
    & (Join-Path $PSScriptRoot 'Prepare-MediaTools.ps1')
    & node (Join-Path $PSScriptRoot 'Build-LicenseBundle.cjs') --check
    if ($LASTEXITCODE -ne 0) { throw 'Dependency license/source bundle verification failed.' }
    if (-not $SkipBuild) {
        & (Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe') build --release --locked --bins
        if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    }
    Assert-ReleaseBinaryVersions -Root $workspaceRoot -Version $version
    if ($signing) {
        foreach ($file in @('de.exe','de-host.exe','de-cli.exe')) {
            Invoke-AuthenticodeSign -Path (Join-Path $workspaceRoot "target\release\$file") -Configuration $signing
        }
    }
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    $compilerArguments = @('/Qp', "/DSourceRoot=$workspaceRoot", "/DAppVersion=$version", "/DReleaseOutputDir=$outputDirectory")
    if ($TestBuild) { $compilerArguments += '/DTestBuild=1' }
    if ($AllowUnsignedRelease) { $compilerArguments += '/DUnsignedRelease=1' }
    if ($signing) {
        $compilerArguments += '/DSignRelease=1'
        $compilerArguments += "/Sidmex=$(New-InnoSignToolCommand -Configuration $signing)"
    }
    $compilerArguments += (Join-Path $workspaceRoot 'installer\DownloadEngine.iss')
    & $Compiler @compilerArguments
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $suffix = if ($TestBuild) { 'TestSetup' } elseif ($AllowUnsignedRelease) { 'UnsignedSetup' } else { 'Setup' }
    $output = Join-Path $outputDirectory "DownloadEngine-$version-$suffix.exe"
    if ($signing) {
        Assert-AuthenticodeSignature -Path $output -Configuration $signing
    }
    Get-FileHash -Algorithm SHA256 -LiteralPath $output
    Write-Output "Installer: $output"
} finally { Pop-Location }
