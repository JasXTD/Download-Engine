[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$IdentityPath,
    [string]$MakeAppx
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Msix.ps1')
. (Join-Path $PSScriptRoot 'Release-Version.ps1')
if (-not $IdentityPath) { $IdentityPath = Join-Path $root 'installer/msix/identity.json' }
$identity = Get-MsixIdentity -Path $IdentityPath
$version = Get-ReleaseVersion -Root $root
if (-not $MakeAppx) {
    $sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $MakeAppx = Get-ChildItem -LiteralPath $sdk -Directory |
        Where-Object { $_.Name -match '^10\.0\.[0-9]+\.[0-9]+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64/makeappx.exe' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $MakeAppx -or -not [IO.Path]::IsPathFullyQualified($MakeAppx) -or
    [IO.Path]::GetFileName($MakeAppx) -ine 'makeappx.exe') {
    throw 'An installed Windows SDK MakeAppx.exe is required. No SDK is downloaded automatically.'
}
$signature = Get-AuthenticodeSignature -LiteralPath $MakeAppx
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|, )O=Microsoft Corporation(,|$)') {
    throw 'MakeAppx must have a valid Microsoft Authenticode signature.'
}

# Reuse the fresh, allowlisted portable payload. Never read the stateful dist app
# folder, install an MSIX, import a certificate, or alter existing release assets.
& (Join-Path $PSScriptRoot 'Package.ps1') -SkipBuild:$SkipBuild -AllowUnsignedDevelopment
$portable = Join-Path $root "dist/DownloadEngine-$version-UnsignedDev-windows-x64.zip"
$outputDirectory = Resolve-ReleasePath -Root $root -Relative 'dist/msix'
$null = New-Item -ItemType Directory -Path $outputDirectory -Force
$stageRelative = 'artifacts/msix-' + [Guid]::NewGuid().ToString('N')
$stage = Resolve-ReleasePath -Root $root -Relative $stageRelative
$null = New-Item -ItemType Directory -Path $stage
try {
    $payload = Join-Path $stage 'payload'
    Copy-MsixPayload -Archive $portable -Destination $payload
    Write-MsixManifest -Template (Join-Path $root 'installer/msix/AppxManifest.xml.in') -Identity $identity -Version $version -Destination (Join-Path $payload 'AppxManifest.xml')
    Write-MsixIcons -Source (Join-Path $root 'assets/download-engine.png') -Destination (Join-Path $payload 'StoreAssets')
    $pending = Join-Path $stage 'validated.msix'
    & $MakeAppx pack /h SHA256 /d $payload /p $pending
    if ($LASTEXITCODE -ne 0) { throw 'MSIX schema validation or packaging failed.' }
    $output = Resolve-ReleasePath -Root $root -Relative "dist/msix/DownloadEngine-$version-x64-StoreUpload.msix"
    $checksums = Resolve-ReleasePath -Root $root -Relative "dist/msix/SHA256SUMS-MSIX-$version"
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pending).Hash.ToLowerInvariant()
    $pendingChecksums = Join-Path $stage 'checksums'
    [IO.File]::WriteAllText($pendingChecksums, "$hash  $([IO.Path]::GetFileName($output))`n", [Text.UTF8Encoding]::new($false))
    # Replace an earlier artifact only after validation succeeds, never while
    # MakeAppx is still writing it. Both paths are on the workspace's volume.
    [IO.File]::Move($pending, $output, $true)
    [IO.File]::Move($pendingChecksums, $checksums, $true)
    Write-Output "Store submission package: $output"
    Write-Output 'This upload package is unsigned. Microsoft signs it only after Store certification; do not present it as a signed public download.'
} finally {
    if (Test-Path -LiteralPath $stage) {
        $resolved = Resolve-ReleasePath -Root $root -Relative $stageRelative
        if ((Split-Path $resolved -Parent) -ne (Join-Path $root 'artifacts') -or
            (Split-Path $resolved -Leaf) -notmatch '^msix-[0-9a-f]{32}$') {
            throw 'Refusing to remove an unexpected MSIX staging directory.'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
