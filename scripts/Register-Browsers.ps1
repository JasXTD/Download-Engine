[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [ValidatePattern('^[a-p]{32}$')][string]$ChromeId,
    [ValidatePattern('^[a-p]{32}$')][string]$EdgeId,
    [ValidatePattern('^[a-p]{32}$')][string]$BraveId,
    [switch]$Firefox,
    [switch]$Unregister
)
$ErrorActionPreference = 'Stop'
$hostExe = Join-Path $PSScriptRoot 'de-host.exe'
if (-not (Test-Path -LiteralPath $hostExe -PathType Leaf)) {
    throw 'Run the copy beside de-host.exe, or use Browser setup inside Download Engine.'
}
if (-not ($ChromeId -or $EdgeId -or $BraveId -or $Firefox)) {
    throw 'Choose -ChromeId <32-letter ID>, -EdgeId <32-letter ID>, -BraveId <32-letter ID>, or -Firefox.'
}
$targets = @()
if ($ChromeId) { $targets += @{ Browser='chrome'; Id=$ChromeId } }
if ($EdgeId) { $targets += @{ Browser='edge'; Id=$EdgeId } }
if ($BraveId) { $targets += @{ Browser='brave'; Id=$BraveId } }
if ($Firefox) { $targets += @{ Browser='firefox'; Id='download-engine@xetro' } }
foreach ($target in $targets) {
    $action = if ($Unregister) { 'Disconnect this copy of Download Engine' } else { 'Register this Download Engine native host for the current user' }
    if ($PSCmdlet.ShouldProcess($target.Browser, $action)) {
        $hostArguments = if ($Unregister) { @('--unregister-browser', $target.Browser) } else { @('--register-browser', $target.Browser, $target.Id) }
        & $hostExe @hostArguments
        if ($LASTEXITCODE -ne 0) { throw "Browser setup failed for $($target.Browser)." }
    }
}
