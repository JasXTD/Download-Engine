[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$SignTool,
    [string]$CertificateThumbprint,
    [string]$TimestampUrl,
    [switch]$AllowUnsignedDevelopment,
    [switch]$AllowUnsignedRelease,
    [switch]$UpdateLocalPortable
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
$null = Get-ReleaseSourceFiles -Root $workspaceRoot
$outputDirectory = Join-Path $workspaceRoot 'dist'
if (-not $AllowUnsignedDevelopment) {
    Assert-PublicReleaseContents -Root $workspaceRoot -Version $version -Signed:([bool]$signing)
    $outputDirectory = Get-PublicReleaseDirectory -Root $workspaceRoot -Version $version
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
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    if ($signing) {
        foreach ($file in @('de.exe','de-host.exe','de-cli.exe')) {
            Invoke-AuthenticodeSign -Path (Join-Path $workspaceRoot "target\release\$file") -Configuration $signing
        }
    }
    $artifactVersion = if ($signing) { $version } elseif ($AllowUnsignedRelease) { "$version-Unsigned" } else { "$version-UnsignedDev" }
    $extensionVersion = if ($AllowUnsignedDevelopment) { "$version-UnsignedDev" } else { $version }
    $localName = if ($signing) { 'DownloadEngine' } else { 'IDM-EX-UnsignedDev' }
    $localDestination = Join-Path $workspaceRoot "dist\$localName"
    if ($UpdateLocalPortable) {
        $running = @(Get-Process -Name 'de','de-host','de-cli','idm-ex','idm-ex-host','idm-ex-cli' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and (Split-Path $_.Path -Parent) -eq $localDestination })
        if ($running.Count) { throw 'Close the local portable Download Engine application before updating its stateful test profile.' }
    }
    # Build archives from a fresh whitelist, never from a user's runtime folder.
    $stageRoot = Join-Path $workspaceRoot 'artifacts'
    New-Item -ItemType Directory -Path $stageRoot -Force | Out-Null
    $stage = Join-Path $stageRoot ('package-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        foreach ($file in @('de.exe','de-host.exe','de-cli.exe')) {
            Copy-Item -LiteralPath (Join-Path $workspaceRoot "target\release\$file") -Destination (Join-Path $stage $file)
        }
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Register-Browsers.ps1') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'README.md') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'SECURITY.md') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'PRIVACY.md') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'CHANGELOG.md') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'LICENSE') -Destination $stage
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'THIRD-PARTY-NOTICES.md') -Destination $stage
        $examplesRoot = Join-Path $stage 'examples'
        New-Item -ItemType Directory -Path $examplesRoot | Out-Null
        foreach ($example in @('core-api-automation.lua','custom-site-reader.lua','download-provider.lua','download-provider-custom-ctr.lua')) {
            Copy-Item -LiteralPath (Join-Path $workspaceRoot "examples/$example") -Destination $examplesRoot
        }
        $mediaRoot = Join-Path $stage 'tools\ffmpeg'
        New-Item -ItemType Directory -Force -Path $mediaRoot | Out-Null
        foreach ($file in @('ffmpeg.exe','LICENSE','README.txt','PROVENANCE.md','media-manifest.json')) {
            Copy-Item -LiteralPath (Join-Path $workspaceRoot "tools\ffmpeg\$file") -Destination $mediaRoot
        }
        foreach ($directory in @('source','licenses')) {
            Copy-Item -LiteralPath (Join-Path $workspaceRoot "tools\ffmpeg\$directory") -Destination $mediaRoot -Recurse
        }
        New-Item -ItemType Directory -Path (Join-Path $stage 'docs') | Out-Null
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\browser-release.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\youtube-port.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\mega.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\wetransfer.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\automation.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\mcp.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\mcp-ai-guide.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\user-guide.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\lua-providers.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\lua-provider-lab.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\releases.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\localization.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\locales') -Destination (Join-Path $stage 'docs') -Recurse
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\licenses') -Destination (Join-Path $stage 'docs') -Recurse
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\github-release.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\reference.md') -Destination (Join-Path $stage 'docs')
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'docs\images') -Destination (Join-Path $stage 'docs') -Recurse
        [System.IO.File]::WriteAllText((Join-Path $stage 'portable.json'), '{"data":"data","downloads":"downloads"}', [System.Text.UTF8Encoding]::new($false))
        foreach ($browserName in @('chrome','edge','brave','firefox')) {
            $extensionRoot = Join-Path $stage "extensions\$browserName"
            New-Item -ItemType Directory -Force -Path $extensionRoot | Out-Null
            foreach ($file in @('i18n-catalog.js','i18n.js','core.js','capture.js','link-recovery.js','page-links.js','browser-login.js','video-button-manager.js','video-button.js','github-download.js','media.js','permission-flow.js','login.html','login.js','background.js','popup.html','popup.css','popup.js','links.html','links.css','links.js')) {
                Copy-Item -LiteralPath (Join-Path $workspaceRoot "extension\$file") -Destination $extensionRoot
            }
            Copy-Item -LiteralPath (Join-Path $workspaceRoot 'extension\_locales') -Destination $extensionRoot -Recurse
            Copy-Item -LiteralPath (Join-Path $workspaceRoot 'extension\icons') -Destination $extensionRoot -Recurse
            Copy-Item -LiteralPath (Join-Path $workspaceRoot 'LICENSE') -Destination $extensionRoot
            Copy-Item -LiteralPath (Join-Path $workspaceRoot 'PRIVACY.md') -Destination $extensionRoot
            $variant = if ($browserName -eq 'firefox') { 'firefox' } else { 'chromium' }
            $manifestSource = Join-Path $workspaceRoot "extension\manifest.$variant.json"
            Copy-Item -LiteralPath $manifestSource -Destination (Join-Path $extensionRoot 'manifest.json')
            # Brave installs from the Chrome Web Store, so it needs its own folder but no separate store package.
            if ($browserName -ne 'brave') {
                Compress-Archive -Path (Join-Path $extensionRoot '*') -DestinationPath (Join-Path $outputDirectory "download-engine-$browserName-$extensionVersion.zip") -Force
            }
        }
        $payload = @('de.exe','de-host.exe','de-cli.exe','portable.json','Register-Browsers.ps1','README.md','SECURITY.md','PRIVACY.md','CHANGELOG.md','LICENSE','THIRD-PARTY-NOTICES.md','extensions','docs','tools','examples') |
            ForEach-Object { Join-Path $stage $_ }
        $portableArchive = Join-Path $outputDirectory "DownloadEngine-$artifactVersion-windows-x64.zip"
        Compress-Archive -LiteralPath $payload -DestinationPath $portableArchive -Force
        if ($UpdateLocalPortable) {
            New-Item -ItemType Directory -Path $localDestination -Force | Out-Null
            foreach ($sourceFile in Get-ChildItem -LiteralPath $stage -File -Recurse) {
                $relative = $sourceFile.FullName.Substring($stage.Length + 1)
                $target = Join-Path $localDestination $relative
                New-Item -ItemType Directory -Force -Path (Split-Path $target -Parent) | Out-Null
                Copy-Item -LiteralPath $sourceFile.FullName -Destination $target -Force
            }
            # Programs from before 2.0; de.exe repoints this profile's browser connections at launch.
            foreach ($legacy in @('idm-ex.exe','idm-ex-host.exe','idm-ex-cli.exe')) {
                $legacyPath = Join-Path $localDestination $legacy
                if (Test-Path -LiteralPath $legacyPath) { Remove-Item -LiteralPath $legacyPath -Force }
            }
            Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $localDestination 'de.exe'),(Join-Path $localDestination 'de-host.exe')
            Write-Warning "Updated stateful local profile: $localDestination. It preserves personal data and downloads; never distribute this directory."
        }
        Get-FileHash -Algorithm SHA256 -LiteralPath $portableArchive
        Write-Output "Clean portable archive: $portableArchive"
    } finally {
        if (Test-Path -LiteralPath $stage) {
            $stagePath = [IO.Path]::GetFullPath($stage)
            $rootPath = [IO.Path]::GetFullPath($stageRoot)
            if ((Split-Path $stagePath -Parent) -eq $rootPath -and (Split-Path $stagePath -Leaf) -match '^package-[0-9a-f]{32}$') {
                Remove-Item -LiteralPath $stagePath -Recurse -Force
            } else {
                Write-Warning "Refusing to clean unexpected package staging path: $stagePath"
            }
        }
    }
} finally { Pop-Location }
