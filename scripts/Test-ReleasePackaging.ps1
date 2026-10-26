[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Release-Files.ps1')
$fixtureRoot = Join-Path $workspaceRoot 'artifacts'
$fixture = Join-Path $fixtureRoot ('release-packaging-test-' + [Guid]::NewGuid().ToString('N'))
$script:passed = 0
function Assert-Check([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:passed++
}
function Expect-Rejection([scriptblock]$Action, [string]$Reason) {
    $failure = $null
    try { & $Action | Out-Null } catch { $failure = $_.Exception.Message }
    Assert-Check ($failure -and $failure.Contains($Reason)) "Expected '$Reason', got '$failure'."
}
function Put-FixtureFile([string]$Relative, [string]$Content) {
    $path = Join-Path $fixture $Relative
    New-Item -ItemType Directory -Path (Split-Path $path -Parent) -Force | Out-Null
    [IO.File]::WriteAllText($path, $Content)
}
function Save-Inventory {
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixture 'scripts/release-source-inventory.json') -Encoding utf8
}
try {
    foreach ($name in @('assets','docs','examples','extension','installer','locales','scripts','src','tests','tools/ffmpeg','vendor')) {
        # Each root needs a real file so it also exists in the staged source tree.
        Put-FixtureFile "$name/README.md" 'Isolated public source fixture.'
    }
    foreach ($name in @('Cargo.toml','Cargo.lock','build.rs','README.md','SECURITY.md','PRIVACY.md','CHANGELOG.md','LICENSE','THIRD-PARTY-NOTICES.md','.gitattributes','.gitignore')) {
        Put-FixtureFile $name 'Isolated public source fixture.'
    }
    foreach ($name in @('Release-Files.ps1','Package-ReleaseSources.ps1','Write-ReleaseChecksums.ps1')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $fixture "scripts/$name")
    }
    Put-FixtureFile 'scripts/Release-Version.ps1' 'function Get-ReleaseVersion { param([string]$Root) return ''9.8.7'' }'
    Put-FixtureFile 'scripts/Prepare-MediaTools.ps1' '# Isolated fixture: no external tools or network.'
    Put-FixtureFile 'scripts/Build-LicenseBundle.cjs' '// Isolated fixture: no dependencies or network.'
    Put-FixtureFile 'src/main.rs' 'fn main() {}'
    $baselineFiles = @(Get-ChildItem -LiteralPath $fixture -File -Recurse -Force | ForEach-Object { $_.FullName.Substring($fixture.Length + 1).Replace('\','/') }) + 'scripts/release-source-inventory.json'
    $manifest = [pscustomobject]@{ schema = 1; files = $baselineFiles; exceptions = @() }
    Save-Inventory
    Assert-Check (@(Get-ReleaseSourceFiles -Root $fixture).Count -eq $baselineFiles.Count) 'Valid source inventory was rejected.'

    foreach ($relative in @('src/.env','docs/.env.local','tests/signing.pfx','assets/signing.p12','src/token.dpapi','examples/queue.sqlite3-wal','docs/session.log','docs/private.pem','src/credentials.json')) {
        Put-FixtureFile $relative 'Synthetic test data, never a real credential.'
        Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Unreviewed source file'
        $manifest.files = @($baselineFiles + $relative); Save-Inventory
        Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Sensitive source filename'
        $manifest.files = $baselineFiles; Save-Inventory
        Remove-Item -LiteralPath (Join-Path $fixture $relative)
    }
    Put-FixtureFile 'src/unreviewed.rs' 'fn unexpected() {}'
    Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Unreviewed source file'
    Remove-Item -LiteralPath (Join-Path $fixture 'src/unreviewed.rs')
    foreach ($extra in @('../outside.rs','/absolute.rs','src/../outside.rs','src//main.rs','src/CON.rs','src/main.rs','SRC/MAIN.RS')) {
        $manifest.files = @($baselineFiles + $extra); Save-Inventory
        $reason = if ($extra -ieq 'src/main.rs') { 'Duplicate source inventory' } else { 'Unsafe release path' }
        Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } $reason
    }
    $manifest.files = @($baselineFiles + 'src/missing.rs'); Save-Inventory
    Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Missing source inventory file'
    $manifest.files = $baselineFiles; Save-Inventory
    foreach ($content in @(
        ('-----BEGIN ' + 'PRIVATE KEY-----' + "`nsynthetic`n"),
        ('{"private_key":"' + '-----BEGIN ' + 'PRIVATE KEY-----\\nsynthetic"}'),
        ('AKIA' + ('A' * 16)),
        ('ghp_' + ('a' * 36))
    )) {
        Put-FixtureFile 'src/main.rs' $content
        Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Sensitive credential content'
    }
    Put-FixtureFile 'src/main.rs' ("SQLite format 3`0" + 'synthetic database')
    Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Sensitive database content'
    Put-FixtureFile 'src/main.rs' 'fn main() {}'

    Put-FixtureFile 'tests/key.cer' ('-----BEGIN ' + 'PRIVATE KEY-----' + "`nsynthetic`n")
    $manifest.files = @($baselineFiles + 'tests/key.cer'); Save-Inventory
    Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'Sensitive credential content'
    $manifest.files = $baselineFiles; Save-Inventory
    Remove-Item -LiteralPath (Join-Path $fixture 'tests/key.cer')

    $identity = 'tests/fixtures/ftp/server.pfx'
    New-Item -ItemType Directory -Path (Join-Path $fixture 'tests/fixtures/ftp') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspaceRoot $identity) -Destination (Join-Path $fixture $identity)
    $manifest.files = @($baselineFiles + $identity)
    $manifest.exceptions = @([pscustomobject]@{ path=$identity; sha256=(Get-FileHash -LiteralPath (Join-Path $fixture $identity) -Algorithm SHA256).Hash.ToLowerInvariant(); reason='Public localhost test identity.' })
    Save-Inventory
    Assert-Check (@(Get-ReleaseSourceFiles -Root $fixture).Count -eq $manifest.files.Count) 'Hash-pinned test identity was rejected.'
    $original = [IO.File]::ReadAllBytes((Join-Path $fixture $identity))
    [IO.File]::WriteAllBytes((Join-Path $fixture $identity), ($original + [byte]0))
    Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'exception checksum mismatch'
    [IO.File]::WriteAllBytes((Join-Path $fixture $identity), $original)

    # A junction is possible without administrator privileges on Windows. It must
    # be rejected before traversal, including when placed in an ignored cache path.
    Put-FixtureFile 'private-outside/source.env' 'Synthetic private fixture.'
    foreach ($relative in @('src/linked','src/node_modules','dist/releases/9.8.8')) {
        $junction = Join-Path $fixture $relative
        New-Item -ItemType Directory -Path (Split-Path $junction -Parent) -Force | Out-Null
        New-Item -ItemType Junction -Path $junction -Target (Join-Path $fixture 'private-outside') | Out-Null
        try {
            if ($relative.StartsWith('src/')) {
                Expect-Rejection { Get-ReleaseSourceFiles -Root $fixture } 'must not contain links'
            } else {
                Expect-Rejection { Assert-PublicReleaseContents -Root $fixture -Version '9.8.8' } 'must not contain links'
            }
        } finally { Remove-Item -LiteralPath $junction -Force }
    }
    Put-FixtureFile '.env' 'Synthetic private top-level file; never an archive candidate.'
    Put-FixtureFile 'AGENTS.md' 'Local contributor guidance; not required or included in release archives.'
    Put-FixtureFile 'scripts/__pycache__/local.pyc' 'Disposable cache; never an archive candidate.'
    & (Join-Path $fixture 'scripts/Package-ReleaseSources.ps1') | Out-Null
    $directory = Get-PublicReleaseDirectory -Root $fixture -Version '9.8.7'
    $archive = Join-Path $directory 'DownloadEngine-9.8.7-Source.zip'
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $names = @($zip.Entries.FullName | ForEach-Object { $_.Replace('\','/') })
        Assert-Check (@(Compare-Object ($manifest.files | Sort-Object) ($names | Sort-Object)).Count -eq 0) 'Source ZIP differs from the exact reviewed inventory.'
        Assert-Check (($names | Select-Object -Unique).Count -eq $names.Count) 'Duplicate archive entries.'
    } finally { $zip.Dispose() }
    $archiveHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
    Put-FixtureFile 'src/.env' 'Synthetic private fixture.'
    Expect-Rejection { & (Join-Path $fixture 'scripts/Package-ReleaseSources.ps1') } 'Unreviewed source file'
    Assert-Check ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ceq $archiveHash) 'Rejected source packaging changed the prior archive.'
    Remove-Item -LiteralPath (Join-Path $fixture 'src/.env')

    Expect-Rejection { Assert-PublicReleaseContents -Root $fixture -Version '9.8.7' -RequireComplete } 'Missing public release artifact'
    foreach ($name in Get-PublicReleaseFileNames -Version '9.8.7') {
        if (-not (Test-Path -LiteralPath (Join-Path $directory $name))) { Put-FixtureFile "dist/releases/9.8.7/$name" "Synthetic release artifact $name" }
    }
    & (Join-Path $fixture 'scripts/Write-ReleaseChecksums.ps1') | Out-Null
    $checksums = Join-Path $directory 'SHA256SUMS-9.8.7'
    $lines = @(Get-Content -LiteralPath $checksums)
    Assert-Check ($lines.Count -eq 6) 'Expected six public asset checksums.'
    foreach ($line in $lines) {
        $hash, $name = $line -split '  ', 2
        Assert-Check ((Get-FileHash -LiteralPath (Join-Path $directory $name) -Algorithm SHA256).Hash -ieq $hash) 'Public artifact checksum mismatch.'
    }
    $checksumHash = (Get-FileHash -LiteralPath $checksums -Algorithm SHA256).Hash
    foreach ($name in @('DownloadEngine-9.8.7-TestSetup.exe','DownloadEngine-9.8.6-UnsignedSetup.exe','DownloadEngine-9.8.7-Setup.exe','.env')) {
        Put-FixtureFile "dist/releases/9.8.7/$name" 'Unexpected public artifact fixture.'
        Expect-Rejection { & (Join-Path $fixture 'scripts/Write-ReleaseChecksums.ps1') } 'Unexpected artifact'
        Expect-Rejection { & (Join-Path $fixture 'scripts/Package-ReleaseSources.ps1') } 'Unexpected artifact'
        Assert-Check ((Get-FileHash -LiteralPath $checksums -Algorithm SHA256).Hash -ceq $checksumHash) 'Rejected checksums changed the existing manifest.'
        Remove-Item -LiteralPath (Join-Path $directory $name)
    }
    New-Item -ItemType Directory -Path (Join-Path $directory 'data') | Out-Null
    Expect-Rejection { Assert-PublicReleaseContents -Root $fixture -Version '9.8.7' } 'Unexpected artifact'
    Remove-Item -LiteralPath (Join-Path $directory 'data')
    Expect-Rejection { Get-PublicReleaseDirectory -Root $fixture -Version '../outside' } 'Invalid public release version'
    Expect-Rejection { Assert-PublicReleaseContents -Root $fixture -Version '9.8.7' -Signed } 'Unexpected artifact'
    foreach ($name in Get-PublicReleaseFileNames -Version '9.8.9' -Signed) { Put-FixtureFile "dist/releases/9.8.9/$name" 'Synthetic signed-mode artifact.' }
    Assert-PublicReleaseContents -Root $fixture -Version '9.8.9' -Signed -RequireComplete
    $script:passed++
    Assert-Check (-not (Test-Path -LiteralPath (Join-Path $fixture 'dist/DownloadEngine-9.8.7-Source.zip'))) 'Public source ZIP was placed beside test artifacts.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($fixtureRoot) -or (Split-Path $resolved -Leaf) -notmatch '^release-packaging-test-[a-f0-9]{32}$') { throw 'Unexpected fixture path; refusing cleanup.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
[pscustomobject]@{ passed=$script:passed; fixtureRemoved=(-not (Test-Path -LiteralPath $fixture)); networkUsed=$false } | ConvertTo-Json
