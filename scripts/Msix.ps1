# MSIX staging helpers. No installation, certificate trust changes or network use.
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Release-Files.ps1')

function Get-MsixIdentity {
    param([Parameter(Mandatory)][string]$Path)
    $identity = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($identity.name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9.-]{2,49}$') {
        throw 'MSIX identity name must be the 3-50 character Package/Identity/Name from Partner Center.'
    }
    foreach ($field in @('publisher', 'publisherDisplayName', 'displayName')) {
        $value = [string]$identity.$field
        if ([string]::IsNullOrWhiteSpace($value) -or $value.Length -gt 256 -or $value -match '[\x00-\x1f\x7f]') {
            throw "Invalid MSIX identity field: $field"
        }
    }
    if (-not $identity.publisher.StartsWith('CN=', [StringComparison]::Ordinal)) {
        throw 'Copy Package/Identity/Publisher exactly from Partner Center (including CN=).'
    }
    $null = [Security.Cryptography.X509Certificates.X500DistinguishedName]::new($identity.publisher)
    return $identity
}

function Write-MsixManifest {
    param(
        [Parameter(Mandatory)][string]$Template,
        [Parameter(Mandatory)]$Identity,
        [Parameter(Mandatory)][string]$Version,
        [Parameter(Mandatory)][string]$Destination
    )
    if ($Version -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -or
        @($Version.Split('.') | Where-Object { [long]$_ -gt 65535 }).Count -or
        [long]$Version.Split('.')[0] -eq 0) {
        throw 'MSIX needs major.minor.patch with a nonzero major version and 16-bit components.'
    }
    $values = @{
        NAME = $Identity.name
        PUBLISHER = $Identity.publisher
        DISPLAY_NAME = $Identity.displayName
        PUBLISHER_DISPLAY_NAME = $Identity.publisherDisplayName
        # The final component is reserved by the Store and must remain zero.
        VERSION = "$Version.0"
    }
    $xml = [IO.File]::ReadAllText($Template)
    foreach ($name in $values.Keys) {
        $xml = $xml.Replace("{{$name}}", [Security.SecurityElement]::Escape([string]$values[$name]))
    }
    if ($xml -match '\{\{[A-Z_]+\}\}') { throw 'Unresolved MSIX manifest placeholder.' }
    # Parse without DTDs or external entities even for locally supplied templates.
    $settings = [Xml.XmlReaderSettings]::new()
    $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $settings.XmlResolver = $null
    $reader = [Xml.XmlReader]::Create([IO.StringReader]::new($xml), $settings)
    try { while ($reader.Read()) {} } finally { $reader.Dispose() }
    [IO.File]::WriteAllText($Destination, $xml, [Text.UTF8Encoding]::new($false))
}

function Copy-MsixPayload {
    param([Parameter(Mandatory)][string]$Archive, [Parameter(Mandatory)][string]$Destination)
    if (Test-Path -LiteralPath $Destination) { throw 'MSIX staging must use a fresh directory.' }
    $allowed = @('de.exe','de-host.exe','de-cli.exe','de-torrent.exe','README.md','SECURITY.md','PRIVACY.md','CHANGELOG.md','LICENSE','THIRD-PARTY-NOTICES.md','extensions','docs','tools','examples','plugins','torrent')
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\','/')
            if ($name -match '(^/|:|(^|/)\.{1,2}(/|$)|//)' -or -not $seen.Add($name)) {
                throw 'Unsafe or duplicate portable archive entry.'
            }
            if ($name -in @('portable.json', 'Register-Browsers.ps1')) { continue }
            if ($name.Split('/')[0] -notin $allowed) { throw "Unexpected MSIX payload entry: $name" }
            # These four public FFmpeg build records are verified against the
            # pinned media manifest by Package.ps1, and belong with its sources.
            $mediaBuildRecord = $name -cmatch '^tools/ffmpeg/source/build-record/(compiler-version|config|ffmpeg-pe|ffprobe-pe)\.log$'
            # Files in our generated archive must not be links or runtime data.
            if ((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000 -or
                $name -match '(?i)(^|/)(data|downloads|\.git|node_modules)(/|$)' -or
                (-not $mediaBuildRecord -and (Test-SensitiveReleaseName $name))) {
                throw "Runtime or linked content cannot enter MSIX: $name"
            }
        }
        $null = New-Item -ItemType Directory -Path $Destination
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\','/')
            if ($name -in @('portable.json', 'Register-Browsers.ps1')) { continue }
            $target = [IO.Path]::GetFullPath((Join-Path $Destination $name))
            $boundary = [IO.Path]::GetFullPath($Destination).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
            if (-not $target.StartsWith($boundary, [StringComparison]::OrdinalIgnoreCase)) { throw 'MSIX entry escapes staging.' }
            if ($name.EndsWith('/')) {
                $null = New-Item -ItemType Directory -Path $target -Force
            } else {
                $null = New-Item -ItemType Directory -Path (Split-Path $target -Parent) -Force
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $false)
            }
        }
    } finally { $zip.Dispose() }
    foreach ($required in @('de.exe','de-host.exe','de-cli.exe','de-torrent.exe','tools/ffmpeg/ffmpeg.exe','tools/ffmpeg/PROVENANCE.md','LICENSE','THIRD-PARTY-NOTICES.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $Destination $required) -PathType Leaf)) {
            throw "Required MSIX payload file is missing: $required"
        }
    }
}

function Write-MsixIcons {
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$Destination)
    Add-Type -AssemblyName System.Drawing
    $null = New-Item -ItemType Directory -Path $Destination -Force
    $original = [Drawing.Image]::FromFile($Source)
    try {
        foreach ($icon in @(@('StoreLogo',50), @('Square44x44Logo',44), @('Square150x150Logo',150))) {
            $size = [int]$icon[1]
            $bitmap = [Drawing.Bitmap]::new($size, $size)
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.Clear([Drawing.Color]::Transparent)
                $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $scale = [Math]::Min($size / $original.Width, $size / $original.Height)
                $width = [int][Math]::Round($original.Width * $scale)
                $height = [int][Math]::Round($original.Height * $scale)
                $graphics.DrawImage($original, [int](($size-$width)/2), [int](($size-$height)/2), $width, $height)
                $bitmap.Save((Join-Path $Destination ($icon[0] + '.png')), [Drawing.Imaging.ImageFormat]::Png)
            } finally { $graphics.Dispose(); $bitmap.Dispose() }
        }
    } finally { $original.Dispose() }
}
