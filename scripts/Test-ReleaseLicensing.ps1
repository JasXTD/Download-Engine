[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Signing.ps1')
. (Join-Path $PSScriptRoot 'Media-Bundle.ps1')
$script:passed = 0
function Expect-Rejection([scriptblock]$Action, [string]$Reason) {
    $failure = $null
    try { & $Action | Out-Null } catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notlike "*$Reason*") { throw "Expected '$Reason', got '$failure'." }
    $script:passed++
}
Expect-Rejection { Resolve-CodeSigningConfiguration } 'Provide all code-signing parameters'
Expect-Rejection { Resolve-CodeSigningConfiguration -AllowUnsignedDevelopment -AllowUnsignedRelease } 'not both'
Expect-Rejection { Resolve-CodeSigningConfiguration -AllowUnsignedRelease -CertificateThumbprint ('a' * 40) } 'Do not combine'
if ($null -ne (Resolve-CodeSigningConfiguration -AllowUnsignedRelease)) { throw 'Unsigned release returned a signing identity.' }
$script:passed++
if ($null -ne (Resolve-CodeSigningConfiguration -AllowUnsignedDevelopment)) { throw 'Unsigned development returned a signing identity.' }
$script:passed++
Expect-Rejection { & (Join-Path $PSScriptRoot 'Build-Installer.ps1') -SkipBuild -AllowUnsignedRelease -TestBuild } 'cannot be combined'
Expect-Rejection { & (Join-Path $PSScriptRoot 'Build-Installer.ps1') -SkipBuild -AllowUnsignedDevelopment } 'requires -TestBuild'

$fixtureRoot = Join-Path $workspaceRoot 'artifacts'
$fixture = Join-Path $fixtureRoot ('licensing-test-' + [Guid]::NewGuid().ToString('N'))
$media = Join-Path $fixture 'tools/ffmpeg'
New-Item -ItemType Directory -Path (Join-Path $media 'source'),(Join-Path $fixture 'src/hls') -Force | Out-Null
try {
    $names = @('ffmpeg.exe','LICENSE','README.txt','PROVENANCE.md','source/Dockerfile','source/build.sh','source/apply-security-fixes.sh','source/test-hdr-metadata.c','source/SHA256SUMS','source/README.md',
        'licenses/x264-COPYING','licenses/LICENSE.md','licenses/mingw-w64-COPYRIGHT','licenses/gcc-runtime-COPYRIGHT',
        'source/build-record/config.mak','source/build-record/toolchain-packages.tsv','source/build-record/binaries.sha256',
        ('source/ffmpeg-' + ('a' * 40) + '.tar.gz'),('source/x264-' + ('b' * 40) + '.tar.gz'))
    $files = @(foreach ($name in $names) {
        $file = Join-Path $media $name
        New-Item -ItemType Directory -Path (Split-Path $file -Parent) -Force | Out-Null
        [IO.File]::WriteAllText($file, "fixture $name")
        [pscustomobject]@{ path = $name; sha256 = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $manifestPath = Join-Path $media 'media-manifest.json'
    $manifest = [pscustomobject]@{ schema = 1; license = 'GPL-3.0-or-later'; ffprobeSha256 = ('c' * 64); files = $files }
    function Save-Manifest { $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding utf8 }
    Save-Manifest
    $runtime = Join-Path $fixture 'src/hls/merge.rs'
    [IO.File]::WriteAllText($runtime, ('const FFMPEG_SHA256: &str = "{0}";' -f $files[0].sha256))
    Assert-MediaBundle -Root $fixture
    $script:passed++
    $notice = Join-Path $media 'README.txt'
    [IO.File]::WriteAllText($notice, 'altered')
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Media checksum mismatch'
    [IO.File]::WriteAllText($notice, 'fixture README.txt')
    $archive = Join-Path $media $names[-1]
    Remove-Item -LiteralPath $archive
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Missing media source/notice/binary'
    [IO.File]::WriteAllText($archive, ('fixture ' + $names[-1]))
    $manifest.files = @($files | Where-Object path -ne 'licenses/x264-COPYING'); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Missing required media manifest entry'
    $manifest.files = @($files | Where-Object path -ne 'source/apply-security-fixes.sh'); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Missing required media manifest entry'
    $manifest.files = @($files | Where-Object path -ne 'source/test-hdr-metadata.c'); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Missing required media manifest entry'
    $manifest.files = @($files | Where-Object path -notlike 'source/x264-*'); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Both pinned FFmpeg and x264'
    $manifest.files = $files; Save-Manifest
    $extra = Join-Path $media 'unexpected.bin'
    [IO.File]::WriteAllText($extra, 'unexpected')
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Unlisted file'
    Remove-Item -LiteralPath $extra
    $manifest.files = @($files + $files[0]); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Duplicate media manifest'
    $manifest.files = @($files + [pscustomobject]@{ path = '../outside'; sha256 = ('d' * 64) }); Save-Manifest
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'Invalid media manifest'
    $manifest.files = $files; Save-Manifest
    [IO.File]::WriteAllText($runtime, ('const FFMPEG_SHA256: &str = "{0}";' -f ('e' * 64)))
    Expect-Rejection { Assert-MediaBundle -Root $fixture } 'hash pin disagree'
    [IO.File]::WriteAllText($runtime, ('const FFMPEG_SHA256: &str = "{0}";' -f $files[0].sha256))
    Expect-Rejection { Assert-MediaBundle -Root $fixture -TestTools } 'Test ffprobe'
    Assert-MediaBundle -Root $fixture
    $script:passed++
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($fixtureRoot) -or (Split-Path $resolved -Leaf) -notmatch '^licensing-test-[a-f0-9]{32}$') { throw 'Unexpected fixture path; refusing cleanup.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
[pscustomobject]@{ passed = $script:passed; fixtureRemoved = -not (Test-Path -LiteralPath $fixture) } | ConvertTo-Json
