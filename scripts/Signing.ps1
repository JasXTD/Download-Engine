Set-StrictMode -Version Latest

function Resolve-CodeSigningConfiguration {
    [CmdletBinding()]
    param(
        [string]$SignTool,
        [string]$CertificateThumbprint,
        [string]$TimestampUrl,
        [switch]$AllowUnsignedDevelopment,
        [switch]$AllowUnsignedRelease
    )

    $provided = @(@($SignTool, $CertificateThumbprint, $TimestampUrl) | Where-Object { $_ })
    if ($AllowUnsignedDevelopment -and $AllowUnsignedRelease) {
        throw 'Choose either -AllowUnsignedDevelopment or -AllowUnsignedRelease, not both.'
    }
    if ($AllowUnsignedDevelopment -or $AllowUnsignedRelease) {
        if ($provided.Count -ne 0) {
            throw 'Do not combine unsigned packaging with code-signing parameters.'
        }
        if ($AllowUnsignedRelease) {
            Write-Warning 'Creating an explicitly unsigned public release. Windows may show an unknown publisher warning.'
        } else {
            Write-Warning 'Creating unsigned development artifacts. Do not distribute them.'
        }
        return $null
    }
    if ($provided.Count -ne 3) {
        throw 'Provide all code-signing parameters, or explicitly choose -AllowUnsignedRelease for public unsigned artifacts or -AllowUnsignedDevelopment for local tests.'
    }
    if (-not [System.IO.Path]::IsPathFullyQualified($SignTool) -or
        -not (Test-Path -LiteralPath $SignTool -PathType Leaf)) {
        throw 'SignTool must be the absolute path to an existing signtool.exe.'
    }
    $resolvedTool = (Resolve-Path -LiteralPath $SignTool).Path
    if ([System.IO.Path]::GetFileName($resolvedTool) -ine 'signtool.exe') {
        throw 'SignTool must point to signtool.exe.'
    }
    if ($resolvedTool.IndexOfAny([char[]]'$"') -ge 0) {
        throw 'SignTool path cannot contain quote or dollar characters.'
    }
    $thumbprint = ($CertificateThumbprint -replace '\s', '').ToUpperInvariant()
    if ($thumbprint -notmatch '^[0-9A-F]{40}$') {
        throw 'CertificateThumbprint must be a 40-character hexadecimal certificate thumbprint.'
    }
    $timestamp = $null
    if (-not [Uri]::TryCreate($TimestampUrl, [UriKind]::Absolute, [ref]$timestamp) -or
        $timestamp.Scheme -ne 'https') {
        throw 'TimestampUrl must be an absolute HTTPS URL.'
    }
    if ($timestamp.AbsoluteUri.IndexOfAny([char[]]'$"') -ge 0) {
        throw 'TimestampUrl cannot contain quote or dollar characters.'
    }
    [pscustomobject]@{
        SignTool = $resolvedTool
        CertificateThumbprint = $thumbprint
        TimestampUrl = $timestamp.AbsoluteUri
    }
}

function Assert-AuthenticodeSignature {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)]$Configuration
    )
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        -not $signature.SignerCertificate -or
        $signature.SignerCertificate.Thumbprint -ine $Configuration.CertificateThumbprint) {
        throw "Authenticode verification failed for $Path. Expected the configured publisher certificate; status was $($signature.Status)."
    }
}

function Invoke-AuthenticodeSign {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)]$Configuration
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Cannot sign missing file: $Path"
    }
    & $Configuration.SignTool sign /sha1 $Configuration.CertificateThumbprint /fd SHA256 /tr $Configuration.TimestampUrl /td SHA256 /d 'Download Engine' $Path
    if ($LASTEXITCODE -ne 0) {
        throw "Authenticode signing failed for $Path."
    }
    Assert-AuthenticodeSignature -Path $Path -Configuration $Configuration
}

function New-InnoSignToolCommand {
    [CmdletBinding()]
    param([Parameter(Mandatory)]$Configuration)
    # Inno Setup replaces $q with a quote and $f with the quoted target filename.
    '$q{0}$q sign /sha1 {1} /fd SHA256 /tr $q{2}$q /td SHA256 /d $qDownload Engine$q $f' -f @(
        $Configuration.SignTool,
        $Configuration.CertificateThumbprint,
        $Configuration.TimestampUrl
    )
}
