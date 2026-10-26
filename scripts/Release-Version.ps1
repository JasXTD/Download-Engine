# Shared preflight for portable and installer releases. Cargo.toml is authoritative.
function Get-ReleaseVersion {
    param([Parameter(Mandatory)][string]$Root)
    $cargo = Get-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Raw -Encoding UTF8
    $package = [regex]::Match($cargo, '(?ms)^\[package\]\r?\n(.*?)(?=^\[|\z)').Groups[1].Value
    $version = [regex]::Match($package, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw 'Cargo.toml must contain a numeric major.minor.patch release version.'
    }
    foreach ($part in $version.Split('.')) {
        [uint16]$component = 0
        if (-not [uint16]::TryParse($part, [ref]$component)) {
            throw 'Release version components must fit in 16 bits for Windows and browser packages.'
        }
    }
    $lock = Get-Content -LiteralPath (Join-Path $Root 'Cargo.lock') -Raw -Encoding UTF8
    $lockedVersion = [regex]::Match($lock, '(?m)^name = "download-engine"\r?\nversion = "([^"]+)"').Groups[1].Value
    if ($lockedVersion -cne $version) { throw 'Version mismatch in the download-engine Cargo.lock entry.' }
    foreach ($variant in @('chromium','firefox')) {
        $manifest = Get-Content -LiteralPath (Join-Path $Root "extension/manifest.$variant.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($manifest.version -cne $version) { throw "Version mismatch in the $variant manifest." }
        if ($variant -eq 'firefox') {
            $gecko = $manifest.browser_specific_settings.gecko
            if ($gecko.id -cne 'download-engine@xetro') {
                throw 'Firefox must use the stable Download Engine add-on ID: download-engine@xetro.'
            }
            $data = $gecko.data_collection_permissions
            if ($data.required -notcontains 'browsingActivity' -or $data.required -notcontains 'websiteContent' -or
                $data.required -contains 'none' -or $data.optional -notcontains 'authenticationInfo') {
                throw 'Firefox data_collection_permissions must disclose native URL/content transfers and optional browser login.'
            }
            # AMO also checks Android's 142 minimum for data consent, even for this desktop package.
            if (-not $gecko.strict_min_version -or [version]$gecko.strict_min_version -lt [version]'142.0') {
                throw 'Firefox 142 or newer is required to pass both data-consent compatibility checks.'
            }
            if ($manifest.browser_specific_settings.PSObject.Properties.Name -contains 'gecko_android') {
                throw 'Do not advertise Android support: the extension requires the Windows desktop companion.'
            }
        }
    }
    $changelog = Get-Content -LiteralPath (Join-Path $Root 'CHANGELOG.md') -Raw -Encoding UTF8
    $release = [regex]::Match($changelog, '(?ms)^## ([^\r\n]+)\r?\n(.*?)(?=^## |\z)')
    $heading = $release.Groups[1].Value
    $dateText = [regex]::Match($heading, '[0-9]{4}-[0-9]{2}-[0-9]{2}$').Value
    [datetime]$date = [datetime]::MinValue
    if (-not [datetime]::TryParseExact($dateText, 'yyyy-MM-dd', [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::None, [ref]$date) -or
        $heading -cne "$version $([char]0x2014) $dateText") {
        throw "The first CHANGELOG.md release heading must be version $version followed by an em dash and a YYYY-MM-DD date."
    }
    $notes = $release.Groups[2].Value -replace '(?m)^#+[^\r\n]*', ''
    if ([string]::IsNullOrWhiteSpace($notes)) { throw 'The latest changelog release must contain release notes.' }
    return $version
}

function Assert-ReleaseBinaryVersions {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Version)
    foreach ($file in @('de.exe','de-host.exe','de-cli.exe')) {
        $binary = Get-Item -LiteralPath (Join-Path $Root "target/release/$file") -ErrorAction Stop
        if ($binary.VersionInfo.FileVersion -cne $Version -or $binary.VersionInfo.ProductVersion -cne $Version) {
            throw "Rebuild ${file}: its Windows version metadata does not match release $Version."
        }
    }
}
