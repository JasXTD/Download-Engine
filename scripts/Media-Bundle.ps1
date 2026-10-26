Set-StrictMode -Version Latest

function Assert-MediaBundle {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Root, [switch]$TestTools)
    $mediaRoot = [IO.Path]::GetFullPath((Join-Path $Root 'tools/ffmpeg'))
    $manifest = Get-Content -LiteralPath (Join-Path $mediaRoot 'media-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($manifest.schema -ne 1 -or $manifest.license -cne 'GPL-3.0-or-later' -or $manifest.files.Count -lt 8) {
        throw 'Unrecognized or incomplete media source manifest.'
    }
    $entries = @(Get-ChildItem -LiteralPath $mediaRoot -Recurse -Force)
    foreach ($entry in $entries) {
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Media bundle must not contain links.' }
    }
    $listed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        if ($file.path -notmatch '^[a-zA-Z0-9_./-]+$' -or $file.path -match '(^|/)\.\.(/|$)' -or [IO.Path]::IsPathRooted($file.path) -or $file.sha256 -notmatch '^[a-f0-9]{64}$') { throw 'Invalid media manifest entry.' }
        if (-not $listed.Add($file.path)) { throw "Duplicate media manifest entry: $($file.path)" }
        $target = Join-Path $mediaRoot $file.path
        if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw "Missing media source/notice/binary: $($file.path)" }
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ine $file.sha256) { throw "Media checksum mismatch: $($file.path)" }
    }
    foreach ($required in @('ffmpeg.exe','LICENSE','README.txt','PROVENANCE.md','source/Dockerfile','source/build.sh','source/apply-security-fixes.sh','source/test-hdr-metadata.c','source/SHA256SUMS','source/README.md',
        'licenses/x264-COPYING','licenses/LICENSE.md','licenses/mingw-w64-COPYRIGHT','licenses/gcc-runtime-COPYRIGHT',
        'source/build-record/config.mak','source/build-record/toolchain-packages.tsv','source/build-record/binaries.sha256')) {
        if (-not $listed.Contains($required)) { throw "Missing required media manifest entry: $required" }
    }
    $archives = @($manifest.files | Where-Object { $_.path -match '^source/(ffmpeg|x264)-[a-f0-9]{40}\.tar\.gz$' })
    if ($archives.Count -ne 2 -or @($archives | Where-Object path -like 'source/ffmpeg-*').Count -ne 1 -or @($archives | Where-Object path -like 'source/x264-*').Count -ne 1) { throw 'Both pinned FFmpeg and x264 source archives are required.' }
    foreach ($entry in $entries) {
        if ($entry.PSIsContainer) { continue }
        $relative = $entry.FullName.Substring($mediaRoot.Length + 1).Replace('\','/')
        if ($relative -eq 'media-manifest.json') { continue }
        if (-not $listed.Contains($relative)) { throw "Unlisted file in media bundle: $relative" }
    }
    $runtimeSource = Get-Content -LiteralPath (Join-Path $Root 'src/hls/merge.rs') -Raw
    $runtimePin = [regex]::Match($runtimeSource, 'const FFMPEG_SHA256: &str = "([a-f0-9]{64})";').Groups[1].Value
    $binaryPin = ($manifest.files | Where-Object path -eq 'ffmpeg.exe').sha256
    if ($runtimePin -cne $binaryPin) { throw 'Media manifest and runtime hash pin disagree.' }
    if ($TestTools) {
        $probe = Join-Path $Root 'artifacts/ffmpeg-tools/ffprobe.exe'
        if (-not (Test-Path -LiteralPath $probe -PathType Leaf) -or (Get-FileHash -LiteralPath $probe -Algorithm SHA256).Hash -ine $manifest.ffprobeSha256) { throw 'Test ffprobe is missing or changed; rebuild it from tools/ffmpeg/source.' }
    }
}
