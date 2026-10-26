$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$DownloadEngine = $env:XDOWNLOADER_CLI
# Names from before 2.0 keep working in existing scripts.
$XDownloader = $DownloadEngine
$Event = Get-Content -LiteralPath $env:XDOWNLOADER_EVENT_FILE -Raw -Encoding UTF8 | ConvertFrom-Json
function Invoke-DownloadEngine {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    $previousPreference = $ErrorActionPreference
    try {
        # Windows PowerShell treats redirected native stderr as ErrorRecords.
        # Read both streams, then use the CLI's exit code and JSON receipt.
        $ErrorActionPreference = 'Continue'
        $resultText = & $DownloadEngine @Arguments --no-start 2>&1 | Out-String
        $commandExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previousPreference }
    if ($commandExit -ne 0) { throw "Download Engine command failed: $resultText" }
    $response = $resultText | ConvertFrom-Json
    # A sample run's "Would ..." line, or the notification just shown.
    if ($null -ne $response.note) { [Console]::Out.WriteLine($response.note) }
    if ($null -ne $response.receipt) { return $response.receipt.result }
    return $response.result
}
Set-Alias -Name Invoke-XDownloader -Value Invoke-DownloadEngine
try {
    $global:LASTEXITCODE = 0
    & $env:XDOWNLOADER_SCRIPT_FILE
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine(($_ | Out-String))
    exit 1
}
