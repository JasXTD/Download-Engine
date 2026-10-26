# Shared filesystem boundaries for source archives and public release assets.
Set-StrictMode -Version Latest

function Resolve-ReleasePath {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Relative)
    if ($Relative -notmatch '^[A-Za-z0-9_.+/-]+$' -or $Relative -match '(^|/)(\.{1,2}|)(/|$)' -or
        $Relative -match '(^|/)(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|/|$)' -or $Relative -match '\.(/|$)') {
        throw "Unsafe release path: $Relative"
    }
    $path = [IO.Path]::GetFullPath($Root)
    foreach ($part in @('') + $Relative.Split('/')) {
        if ($part) { $path = Join-Path $path $part }
        $entry = Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        if ($entry -and ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Release paths must not contain links: $Relative"
        }
    }
    return $path
}

function Test-SensitiveReleaseName {
    param([Parameter(Mandatory)][string]$Relative)
    return $Relative -match '(?i)(^|/)\.env($|\.)|\.(pfx|p12|pem|key|dpapi|db|sqlite[0-9]*)([-.]|$)|\.log($|\.)|(^|/)(id_rsa|id_dsa|id_ecdsa|id_ed25519|credentials|secrets)(\.|$)'
}

function Assert-ReleaseSourceContent {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Relative)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $header = [byte[]]::new(16)
        $length = $stream.Read($header, 0, $header.Length)
        if ($length -eq 16 -and [Text.Encoding]::ASCII.GetString($header) -ceq "SQLite format 3`0") {
            throw "Sensitive database content in source file: $Relative"
        }
    } finally { $stream.Dispose() }
    # These reviewed binary assets are not text. Dependency archives have separate
    # Cargo/media checksum verification; sensitive test identities are pinned below.
    if ($Relative -match '\.(pfx|png|ico|zip|gz|crate)$') { return }
    if ((Get-Item -LiteralPath $Path).Length -gt 16MB) { throw "Source text exceeds the review limit: $Relative" }
    $content = [IO.File]::ReadAllText($Path)
    $privateKey = '(?m)^\s*-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----|"private_key"\s*:\s*"-----BEGIN '
    $credential = '(?<![A-Za-z0-9])(?:AKIA|ASIA)[A-Z0-9]{16}(?![A-Za-z0-9])|\bgh[pousr]_[A-Za-z0-9]{36}\b|\bgithub_pat_[A-Za-z0-9_]{82}\b'
    if ($content -match $privateKey -or $content -match $credential) {
        # Report only the path, never the matched secret.
        throw "Sensitive credential content in source file: $Relative"
    }
}

function Get-ReleaseSourceFiles {
    param([Parameter(Mandatory)][string]$Root)
    $inventoryPath = Resolve-ReleasePath -Root $Root -Relative 'scripts/release-source-inventory.json'
    $inventory = Get-Content -LiteralPath $inventoryPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($inventory.schema -ne 1 -or @($inventory.files).Count -eq 0) { throw 'Invalid source release inventory.' }
    $roots = @('assets','docs','examples','extension','installer','locales','scripts','src','tests','tools/ffmpeg','vendor')
    $topFiles = @('Cargo.toml','Cargo.lock','build.rs','README.md','SECURITY.md','CHANGELOG.md','LICENSE','THIRD-PARTY-NOTICES.md','.gitattributes','.gitignore')
    $listed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $inventory.files) {
        $null = Resolve-ReleasePath -Root $Root -Relative $relative
        if (-not $listed.Add($relative)) { throw "Duplicate source inventory entry: $relative" }
        if ($relative -notin $topFiles -and -not @($roots | Where-Object { $relative.StartsWith("$_/", [StringComparison]::Ordinal) }).Count) {
            throw "Source inventory entry outside approved roots: $relative"
        }
        if ($relative -match '(^|/)(__pycache__|\.git|node_modules|\.venv|venv)(/|$)|\.(pyc|pyo|exe|dll|pdb)$') {
            throw "Build/cache file cannot enter source inventory: $relative"
        }
    }
    $exceptions = @{}
    foreach ($exception in $inventory.exceptions) {
        if (-not $listed.Contains($exception.path) -or $exceptions.ContainsKey($exception.path) -or
            $exception.sha256 -cnotmatch '^[a-f0-9]{64}$' -or [string]::IsNullOrWhiteSpace($exception.reason) -or
            -not (Test-SensitiveReleaseName $exception.path)) {
            throw 'Invalid sensitive-file exception in source inventory.'
        }
        $exceptions[$exception.path] = $exception.sha256
    }
    # Traverse one directory at a time so a reparse point is rejected before descent.
    # Top-level private data/build folders are never candidates for a source archive.
    $pending = [Collections.Generic.Queue[string]]::new()
    foreach ($rootName in $roots) { $pending.Enqueue($rootName) }
    $actual = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $topFiles) {
        $file = Resolve-ReleasePath -Root $Root -Relative $relative
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing source file: $relative" }
        $null = $actual.Add($relative)
    }
    while ($pending.Count) {
        $directory = $pending.Dequeue()
        $directoryPath = Resolve-ReleasePath -Root $Root -Relative $directory
        if (-not (Test-Path -LiteralPath $directoryPath -PathType Container)) { throw "Missing source directory: $directory" }
        foreach ($entry in Get-ChildItem -LiteralPath $directoryPath -Force) {
            $relative = "$directory/$($entry.Name)"
            $null = Resolve-ReleasePath -Root $Root -Relative $relative
            if ($entry.PSIsContainer) {
                if ($entry.Name -notin @('__pycache__','.git','node_modules','.venv','venv')) { $pending.Enqueue($relative) }
            } elseif ($relative -cne 'tools/ffmpeg/ffmpeg.exe') {
                $null = $actual.Add($relative)
            }
        }
    }
    foreach ($relative in $actual) {
        if (-not $listed.Contains($relative)) { throw "Unreviewed source file; update the inventory only after review: $relative" }
    }
    foreach ($relative in $inventory.files) {
        if (-not $actual.Contains($relative)) { throw "Missing source inventory file: $relative" }
        $file = Resolve-ReleasePath -Root $Root -Relative $relative
        if (Test-SensitiveReleaseName $relative) {
            if (-not $exceptions.ContainsKey($relative)) { throw "Sensitive source filename needs a reviewed hash exception: $relative" }
            if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ine $exceptions[$relative]) {
                throw "Sensitive source exception checksum mismatch: $relative"
            }
        }
        Assert-ReleaseSourceContent -Path $file -Relative $relative
    }
    return $inventory.files
}

function Get-PublicReleaseDirectory {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Version)
    if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Invalid public release version.' }
    return Resolve-ReleasePath -Root $Root -Relative "dist/releases/$Version"
}

function Get-PublicReleaseFileNames {
    param([Parameter(Mandatory)][string]$Version, [switch]$Signed)
    $portable = if ($Signed) { "DownloadEngine-$Version-windows-x64.zip" } else { "DownloadEngine-$Version-Unsigned-windows-x64.zip" }
    $installer = if ($Signed) { "DownloadEngine-$Version-Setup.exe" } else { "DownloadEngine-$Version-UnsignedSetup.exe" }
    return @($portable,$installer,"download-engine-chrome-$Version.zip","download-engine-edge-$Version.zip","download-engine-firefox-$Version.zip","DownloadEngine-$Version-Source.zip") | Sort-Object
}

function Assert-PublicReleaseContents {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Version, [switch]$Signed, [switch]$RequireComplete)
    $directory = Get-PublicReleaseDirectory -Root $Root -Version $Version
    $names = @(Get-PublicReleaseFileNames -Version $Version -Signed:$Signed)
    if (Test-Path -LiteralPath $directory) {
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) { throw 'Public release path is not a directory.' }
        foreach ($entry in Get-ChildItem -LiteralPath $directory -Force) {
            $null = Resolve-ReleasePath -Root $Root -Relative "dist/releases/$Version/$($entry.Name)"
            if ($entry.PSIsContainer -or $entry.Name -cnotin ($names + "SHA256SUMS-$Version")) {
                throw "Unexpected artifact in public release directory: $($entry.Name)"
            }
        }
    }
    if ($RequireComplete) {
        foreach ($name in $names) {
            if (-not (Test-Path -LiteralPath (Join-Path $directory $name) -PathType Leaf)) { throw "Missing public release artifact: $name" }
        }
    }
}
