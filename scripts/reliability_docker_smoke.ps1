param(
    [string]$BinaryDirectory = 'dist/IDM-EX-UnsignedDev',
    [ValidateSet('large','combined')][string]$Mode = 'large',
    [Parameter(Mandatory=$true)][string]$Node,
    [Parameter(Mandatory=$true)][string]$ReportDirectory
)
$ErrorActionPreference = 'Stop'
$source = (Resolve-Path (Join-Path $PSScriptRoot 'fixtures/reliability_server.go')).Path
$name = 'download-engine-reliability-' + [guid]::NewGuid().ToString('N').Substring(0,12)
$previousOrigin = $env:IDM_EX_RELIABILITY_FIXTURE
$container = $null
try {
    # Copy only our fixture into tmpfs; no host folder sharing or host mount is required.
    $created = & docker run --detach --pull never --name $name --memory 1536m --cpus 2 --read-only --cap-drop ALL --security-opt no-new-privileges --tmpfs '/tmp:rw,exec,size=512m,mode=1777' --env GOCACHE=/tmp/cache --env GOPROXY=off --env GOSUMDB=off --env GOFLAGS=-p=2 --env "FIXTURE_MODE=$Mode" --publish '127.0.0.1::8080' golang:1.27.2-bookworm sh -c 'while [ ! -f /tmp/ready ]; do sleep 0.1; done; exec go run /tmp/main.go'
    $container = "$created".Trim()
    if ($LASTEXITCODE -ne 0 -or $container -notmatch '^[a-f0-9]{64}$') { throw 'Could not start the owned fixture container' }
    Get-Content -LiteralPath $source -Raw | & docker exec --interactive $container sh -c 'cat > /tmp/main.go'
    if ($LASTEXITCODE -ne 0) { throw 'Could not copy the fixture source' }
    & docker exec $container touch /tmp/ready
    if ($LASTEXITCODE -ne 0) { throw 'Could not release the fixture start gate' }
    $mapping = (& docker port $container 8080/tcp).Trim()
    if ($LASTEXITCODE -ne 0 -or $mapping -notmatch '^127\.0\.0\.1:\d+$') { throw 'Fixture must be bound only to loopback' }
    $env:IDM_EX_RELIABILITY_FIXTURE = "http://$mapping"
    $ready = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    while ([DateTime]::UtcNow -lt $deadline) {
        $running = & docker inspect --format '{{.State.Running}}' $container
        if ($running -ne 'true') { throw 'Docker fixture exited; inspect the saved server log' }
        try { $null = Invoke-RestMethod "$env:IDM_EX_RELIABILITY_FIXTURE/health" -TimeoutSec 2; $ready = $true; break } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $ready) { throw 'Docker fixture did not become ready' }
    & $Node (Join-Path $PSScriptRoot 'reliability_packaged_smoke.cjs') $BinaryDirectory "--$Mode" *> (Join-Path $ReportDirectory "docker-$Mode.log")
    $testExit = $LASTEXITCODE
    Get-Content (Join-Path $ReportDirectory "docker-$Mode.log") -Tail 35
    if ($testExit -ne 0) { throw "Packaged $Mode test failed with exit $testExit" }
} finally {
    if ($container -match '^[a-f0-9]{64}$') {
        & docker logs $container *> (Join-Path $ReportDirectory "docker-$Mode-server.log")
        & docker rm --force $container | Out-Null
    }
    $env:IDM_EX_RELIABILITY_FIXTURE = $previousOrigin
}
