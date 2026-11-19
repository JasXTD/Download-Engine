[CmdletBinding()]
param([string]$MakeAppx)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Msix.ps1')
$testRoot = Join-Path $root ('artifacts/msix-tests-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$script:passed = 0
function Assert-Msix($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Test-MsixCase([string]$Name, [scriptblock]$Body) {
    & $Body
    $script:passed++
    Write-Output "PASS $Name"
}
function Assert-MsixRejected([scriptblock]$Body, [string]$Message) {
    $rejected = $false
    try { & $Body } catch { $rejected = $true }
    Assert-Msix $rejected $Message
}
function New-MsixFixtureArchive([string]$Name, [string[]]$Extra = @()) {
    $archive = Join-Path $testRoot "$Name.zip"
    $zip = [IO.Compression.ZipFile]::Open($archive, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($entryName in @('de.exe','de-host.exe','de-cli.exe','de-torrent.exe','portable.json','Register-Browsers.ps1','tools/ffmpeg/ffmpeg.exe','tools/ffmpeg/PROVENANCE.md','LICENSE','THIRD-PARTY-NOTICES.md','plugins/example.de-extension.json') + $Extra) {
            $entry = $zip.CreateEntry($entryName)
            $writer = [IO.StreamWriter]::new($entry.Open())
            try { $writer.Write('offline packaging fixture - never executable') } finally { $writer.Dispose() }
        }
    } finally { $zip.Dispose() }
    return $archive
}
try {
    $identityPath = Join-Path $root 'installer/msix/identity.json'
    $template = Join-Path $root 'installer/msix/AppxManifest.xml.in'
    $identity = Get-MsixIdentity $identityPath
    Test-MsixCase 'Partner Center identity, escaping and Store version' {
        $manifestPath = Join-Path $testRoot 'AppxManifest.xml'
        $escaped = $identity.PSObject.Copy()
        $escaped.publisherDisplayName = 'Publisher & <Friends>'
        Write-MsixManifest $template $escaped '3.20.1' $manifestPath
        [xml]$manifest = Get-Content -LiteralPath $manifestPath -Raw
        Assert-Msix ($manifest.Package.Identity.Name -ceq 'TwinCatz.DownloadEngine') 'Package name changed'
        Assert-Msix ($manifest.Package.Identity.Publisher -ceq $identity.publisher) 'Publisher changed'
        Assert-Msix ($manifest.Package.Identity.Version -ceq '3.20.1.0') 'Store version is invalid'
        Assert-Msix ($manifest.Package.Properties.PublisherDisplayName -ceq 'Publisher & <Friends>') 'Identity text was not XML-escaped'
    }
    Test-MsixCase 'Invalid identities and versions are rejected' {
        $badIdentity = Join-Path $testRoot 'bad-identity.json'
        $bad = $identity.PSObject.Copy()
        foreach ($name in @('../outside','ab','Bad Name',('a' * 51))) {
            $bad.name = $name
            $bad | ConvertTo-Json | Set-Content -LiteralPath $badIdentity
            Assert-MsixRejected { Get-MsixIdentity $badIdentity } "Accepted identity $name"
        }
        foreach ($version in @('3.20','3.20.1.1','03.20.1','0.1.0','3.20.65536','3.20.1-beta')) {
            Assert-MsixRejected { Write-MsixManifest $template $identity $version (Join-Path $testRoot 'bad.xml') } "Accepted version $version"
        }
    }
    Test-MsixCase 'Manifest retains review arguments and opt-in startup' {
        [xml]$manifest = Get-Content -LiteralPath (Join-Path $testRoot 'AppxManifest.xml') -Raw
        $startup = $manifest.SelectSingleNode("//*[local-name()='StartupTask']")
        Assert-Msix ($startup.Enabled -ceq 'false') 'Startup must be off by default'
        Assert-Msix ($startup.TaskId -ceq 'DownloadEngineStartup') 'Startup API/manifest mismatch'
        Assert-Msix ($startup.ParentNode.GetAttribute('Parameters', 'http://schemas.microsoft.com/appx/manifest/uap/windows10/10') -ceq '--autostart') 'Missing startup launch flag'
        foreach ($kind in @('FileTypeAssociation','Protocol')) {
            $handler = $manifest.SelectSingleNode("//*[local-name()='$kind']")
            Assert-Msix ($handler.Parameters -ceq '--open "%1"') 'Shell input must be passed to the existing review parser as one quoted argument'
        }
        $alias = $manifest.SelectSingleNode("//*[local-name()='ExecutionAlias']")
        Assert-Msix ($alias.Alias -ceq 'download-engine-host.exe') 'Browser host alias mismatch'
        Assert-Msix ($alias.ParentNode.ParentNode.Executable -ceq 'de-host.exe') 'Browser alias must start the real host'
        $keys = $manifest.SelectNodes("//*[local-name()='ExcludedKey']")
        Assert-Msix ($keys.Count -eq 4) 'Virtualization exclusions must remain narrow on Windows 11'
        foreach ($key in $keys) {
            Assert-Msix ($key.InnerText.EndsWith('\NativeMessagingHosts\com.xetro.downloadengine')) 'Unexpected externally visible registry key'
        }
        $capabilities = @($manifest.Package.Capabilities.ChildNodes | ForEach-Object { $_.Name })
        Assert-Msix ($capabilities.Count -eq 2 -and $capabilities -contains 'runFullTrust' -and $capabilities -contains 'unvirtualizedResources') 'Unexpected package capabilities'
    }
    Test-MsixCase 'Store supplies the Microsoft desktop C runtime' {
        [xml]$manifest = Get-Content -LiteralPath (Join-Path $testRoot 'AppxManifest.xml') -Raw
        $runtime = $manifest.Package.Dependencies.PackageDependency
        Assert-Msix ($runtime.Name -ceq 'Microsoft.VCLibs.140.00.UWPDesktop') 'Desktop runtime dependency missing'
        Assert-Msix ($runtime.MinVersion -ceq '14.0.30704.0') 'Unexpected runtime floor'
        Assert-Msix ($runtime.Publisher -ceq 'CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US') 'Runtime must come from Microsoft'
    }
    Test-MsixCase 'Fresh payload retains media/plugins but strips portable setup' {
        $archive = New-MsixFixtureArchive 'clean' @('tools/ffmpeg/source/build-record/compiler-version.log')
        $stage = Join-Path $testRoot 'layout'
        Copy-MsixPayload $archive $stage
        Assert-Msix (-not (Test-Path -LiteralPath (Join-Path $stage 'portable.json'))) 'Portable profile would write into WindowsApps'
        Assert-Msix (-not (Test-Path -LiteralPath (Join-Path $stage 'Register-Browsers.ps1'))) 'Unpackaged registration script must not ship in MSIX'
        Assert-Msix (Test-Path -LiteralPath (Join-Path $stage 'tools/ffmpeg/ffmpeg.exe')) 'FFmpeg missing'
        Assert-Msix (Test-Path -LiteralPath (Join-Path $stage 'plugins/example.de-extension.json')) 'Bundled extension missing'
        Assert-Msix (Test-Path -LiteralPath (Join-Path $stage 'tools/ffmpeg/source/build-record/compiler-version.log')) 'Public, pinned media build record missing'
        Assert-MsixRejected { Copy-MsixPayload $archive $stage } 'Staging must not reuse a runtime directory'
    }
    Test-MsixCase 'Traversal, duplicates and private data never enter MSIX' {
        $index = 0
        foreach ($entry in @('../escaped','docs/../../escaped','data/queue.sqlite3','docs/session.dpapi','docs/key.pfx','docs/session.log','tools/ffmpeg/source/build-record/private.log','DE.EXE','docs/../escaped','docs/profile.db','docs/queue.sqlite3-wal','examples/.env.local','plugins/credentials.json')) {
            $index++
            $archive = New-MsixFixtureArchive "unsafe-$index" @($entry)
            $stage = Join-Path $testRoot "unsafe-$index"
            Assert-MsixRejected { Copy-MsixPayload $archive $stage } "Accepted unsafe entry $entry"
            Assert-Msix (-not (Test-Path -LiteralPath $stage)) 'Rejected archive wrote partial output'
        }
        Assert-Msix (-not (Test-Path -LiteralPath (Join-Path $root 'escaped'))) 'Archive escaped staging'
    }
    Test-MsixCase 'Generated Store icons have the required dimensions' {
        $icons = Join-Path $testRoot 'icons'
        Write-MsixIcons (Join-Path $root 'assets/download-engine.png') $icons
        foreach ($item in @(@('StoreLogo',50),@('Square44x44Logo',44),@('Square150x150Logo',150))) {
            $image = [Drawing.Image]::FromFile((Join-Path $icons ($item[0] + '.png')))
            try { Assert-Msix ($image.Width -eq $item[1] -and $image.Height -eq $item[1]) 'Incorrect icon dimensions' } finally { $image.Dispose() }
        }
    }
    if ($MakeAppx) {
        Test-MsixCase 'Microsoft SDK accepts the complete package manifest' {
            $signature = Get-AuthenticodeSignature -LiteralPath $MakeAppx
            Assert-Msix ($signature.Status -eq 'Valid' -and $signature.SignerCertificate.Subject -match '(^|, )O=Microsoft Corporation(,|$)') 'MakeAppx is not Microsoft-signed'
            $layout = Join-Path $testRoot 'layout'
            Write-MsixManifest $template $identity '3.20.1' (Join-Path $layout 'AppxManifest.xml')
            Write-MsixIcons (Join-Path $root 'assets/download-engine.png') (Join-Path $layout 'StoreAssets')
            $messages = & $MakeAppx pack /o /h SHA256 /d $layout /p (Join-Path $testRoot 'fixture.msix') 2>&1
            Assert-Msix ($LASTEXITCODE -eq 0) ($messages -join [Environment]::NewLine)
            $zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $testRoot 'fixture.msix'))
            try {
                Assert-Msix ($null -ne $zip.GetEntry('AppxBlockMap.xml')) 'SDK did not create a package block map'
                Assert-Msix ($null -ne $zip.GetEntry('AppxManifest.xml')) 'Package manifest missing'
                Assert-Msix ($null -eq $zip.GetEntry('AppxSignature.p7x')) 'Upload/test fixture must not pretend to be signed'
            } finally { $zip.Dispose() }
        }
    }
    Write-Output "$script:passed MSIX packaging checks passed. No packages or certificates were installed."
} finally {
    $resolved = [IO.Path]::GetFullPath($testRoot)
    if ((Split-Path $resolved -Parent) -ne (Join-Path $root 'artifacts') -or
        (Split-Path $resolved -Leaf) -notmatch '^msix-tests-[0-9a-f]{32}$') {
        throw 'Unexpected MSIX test cleanup path.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
