[CmdletBinding()]
param([switch]$CheckBinaries)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Release-Version.ps1')
$version = Get-ReleaseVersion -Root $workspaceRoot
$fixtureRelative = 'artifacts/release-version-test-' + [Guid]::NewGuid().ToString('N')
$fixture = & {
    # Keep the path helper's strict mode local to filesystem validation.
    . (Join-Path $PSScriptRoot 'Release-Files.ps1')
    Resolve-ReleasePath -Root $workspaceRoot -Relative $fixtureRelative
}
try {
    New-Item -ItemType Directory -Path (Join-Path $fixture 'extension') -Force | Out-Null
    foreach ($file in @('Cargo.toml','Cargo.lock','CHANGELOG.md','extension/manifest.chromium.json','extension/manifest.firefox.json')) {
        Copy-Item -LiteralPath (Join-Path $workspaceRoot $file) -Destination (Join-Path $fixture $file)
    }
    $script:checks = 0
    function Expect-Failure {
        param([scriptblock]$Action, [string]$Expected)
        $message = $null
        try { & $Action | Out-Null } catch { $message = $_.Exception.Message }
        if (-not $message -or $message -notlike "*$Expected*") {
            throw "Expected failure containing '$Expected'; got '$message'."
        }
        $script:checks++
    }
    function With-InvalidFile {
        param([string]$RelativePath, [string]$Content, [string]$Expected)
        $path = Join-Path $fixture $RelativePath
        $original = [IO.File]::ReadAllBytes($path)
        try {
            [IO.File]::WriteAllText($path, $Content)
            Expect-Failure { Get-ReleaseVersion -Root $fixture } $Expected
        } finally { [IO.File]::WriteAllBytes($path, $original) }
    }
    if ((Get-ReleaseVersion -Root $fixture) -cne $version) { throw 'Valid release was not accepted.' }
    $script:checks++
    With-InvalidFile 'Cargo.toml' "[package]`nname = `"download-engine`"`nversion = `"1.0`"`n" 'numeric major.minor.patch'
    With-InvalidFile 'Cargo.toml' "[package]`nname = `"download-engine`"`nversion = `"01.0.1`"`n" 'numeric major.minor.patch'
    With-InvalidFile 'Cargo.toml' "[package]`nname = `"download-engine`"`nversion = `"65536.0.1`"`n" '16 bits'
    With-InvalidFile 'Cargo.lock' "[[package]]`nname = `"download-engine`"`nversion = `"0.0.0`"`n" 'Cargo.lock'
    foreach ($variant in @('chromium','firefox')) {
        With-InvalidFile "extension/manifest.$variant.json" '{"version":"0.0.0"}' "$variant manifest"
    }
    $firefox = Get-Content (Join-Path $fixture 'extension/manifest.firefox.json') -Raw | ConvertFrom-Json
    $firefox.browser_specific_settings.gecko.id = 'idm-ex@local'
    With-InvalidFile 'extension/manifest.firefox.json' ($firefox | ConvertTo-Json -Depth 8) 'stable Download Engine add-on ID'
    $firefox = Get-Content (Join-Path $fixture 'extension/manifest.firefox.json') -Raw | ConvertFrom-Json
    $firefox.browser_specific_settings.gecko.PSObject.Properties.Remove('data_collection_permissions')
    With-InvalidFile 'extension/manifest.firefox.json' ($firefox | ConvertTo-Json -Depth 8) 'Firefox data_collection_permissions'
    $firefox = Get-Content (Join-Path $fixture 'extension/manifest.firefox.json') -Raw | ConvertFrom-Json
    $firefox.browser_specific_settings.gecko.data_collection_permissions.required = @('none')
    With-InvalidFile 'extension/manifest.firefox.json' ($firefox | ConvertTo-Json -Depth 8) 'Firefox data_collection_permissions'
    foreach ($minimum in @('128.0', '140.0', '141.0')) {
        $firefox = Get-Content (Join-Path $fixture 'extension/manifest.firefox.json') -Raw | ConvertFrom-Json
        $firefox.browser_specific_settings.gecko.strict_min_version = $minimum
        With-InvalidFile 'extension/manifest.firefox.json' ($firefox | ConvertTo-Json -Depth 8) 'Firefox 142'
    }
    $firefox = Get-Content (Join-Path $fixture 'extension/manifest.firefox.json') -Raw | ConvertFrom-Json
    $firefox.browser_specific_settings | Add-Member -NotePropertyName gecko_android -NotePropertyValue @{}
    With-InvalidFile 'extension/manifest.firefox.json' ($firefox | ConvertTo-Json -Depth 8) 'Windows desktop companion'
    With-InvalidFile 'CHANGELOG.md' "# Changelog`n`n## 0.0.0 $([char]0x2014) 2026-10-16`n`nOld notes." 'first CHANGELOG.md release heading'
    With-InvalidFile 'CHANGELOG.md' "# Changelog`n`n## $version $([char]0x2014) 2026-02-30`n`nNotes." 'first CHANGELOG.md release heading'
    With-InvalidFile 'CHANGELOG.md' "# Changelog`n`n## $version $([char]0x2014) 2026-10-16`n`n### Added`n`n" 'must contain release notes'
    New-Item -ItemType Directory -Path (Join-Path $fixture 'target/release') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $fixture 'target/release/de.exe'), 'No version resource')
    Expect-Failure { Assert-ReleaseBinaryVersions -Root $fixture -Version $version } 'Rebuild de.exe'
    if ($CheckBinaries) {
        Assert-ReleaseBinaryVersions -Root $workspaceRoot -Version $version
        $script:checks++
        Expect-Failure { Assert-ReleaseBinaryVersions -Root $workspaceRoot -Version '0.0.0' } 'Rebuild de.exe'
        $cli = Join-Path $workspaceRoot 'target/release/de-cli.exe'
        $output = & $cli --version
        if ($LASTEXITCODE -ne 0 -or $output -cne "Download Engine $version") { throw "Incorrect CLI version: $output" }
        $script:checks++
        $help = & $cli --help
        if ($LASTEXITCODE -ne 0 -or $help[0] -cne "Download Engine CLI $version") { throw "Incorrect CLI help version: $($help[0])" }
        $script:checks++
    }
} finally {
    & {
        . (Join-Path $PSScriptRoot 'Release-Files.ps1')
        $cleanupPath = Resolve-ReleasePath -Root $workspaceRoot -Relative $fixtureRelative
        if (Test-Path -LiteralPath $cleanupPath) {
            Remove-Item -LiteralPath $cleanupPath -Recurse -Force
        }
    }
}
[pscustomobject]@{
    passed = $script:checks
    version = $version
    binaryChecks = [bool]$CheckBinaries
    fixtureRemoved = -not (Test-Path -LiteralPath $fixture)
} | ConvertTo-Json
