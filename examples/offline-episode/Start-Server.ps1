[CmdletBinding()]
param(
    [ValidateRange(0,65535)][int]$Port = 0,
    [ValidateRange(0,250)][int]$ChunkDelayMs = 0
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# A deliberately small fixture server: only the builder's named files, only
# IPv4 loopback, no directory browsing, uploads, CORS or profile access.
$fixtureRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'fixtures'))
$routeFile = Join-Path $fixtureRoot 'routes.json'
if (-not (Test-Path -LiteralPath $routeFile -PathType Leaf)) {
    throw 'Sample files are missing. Extract the complete demo ZIP, or run the builder described in README.md.'
}
if ((Get-Item -LiteralPath $fixtureRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Fixture root must not be a link.' }
if ((Get-Item -LiteralPath $routeFile).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Fixture route list must not be a link.' }
if ((Get-Item -LiteralPath $routeFile).Length -gt 256KB) { throw 'Fixture route list is too large.' }
$routes = @{}
$totalBytes = 0L
foreach ($item in @(Get-Content -LiteralPath $routeFile -Raw -Encoding UTF8 | ConvertFrom-Json)) {
    $relative = [string]$item.file
    if ($relative -cnotmatch '^assets/[a-z0-9_-]+/(init\.mp4|[0-9]{5}\.enc|subtitles\.srt)$' -or
        [string]$item.url -cne "/$relative" -or $routes.ContainsKey([string]$item.url)) { throw 'Invalid or duplicate fixture route.' }
    $candidate = $fixtureRoot
    foreach ($part in $relative.Split('/')) {
        $candidate = Join-Path $candidate $part
        if ((Get-Item -LiteralPath $candidate).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Fixture paths must not contain links.' }
    }
    $file = Get-Item -LiteralPath $candidate
    if ($file.PSIsContainer -or $file.Length -ne $item.size -or $file.Length -gt 2MB -or
        (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash -ine $item.sha256) { throw 'Sample fixture integrity check failed.' }
    $totalBytes += $file.Length
    if ($routes.Count -ge 128 -or $totalBytes -gt 16MB) { throw 'Fixture bundle exceeds the demo limit.' }
    $routes[[string]$item.url] = [IO.File]::ReadAllBytes($candidate)
}
if ($routes.Count -eq 0) { throw 'No sample routes were found.' }

$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
try {
    $listener.Start(8)
    $actualPort = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    Write-Output "READY http://127.0.0.1:$actualPort/sample-episode"
    Write-Output 'Paste that URL into the installed offline episode extension. Press Ctrl+C to stop.'
    while ($true) {
        $client = $listener.AcceptTcpClient()
        try {
            $client.ReceiveTimeout = 2000
            $client.SendTimeout = 2000
            $stream = $client.GetStream()
            $header = [Collections.Generic.List[byte]]::new()
            $complete = $false
            $headerClock = [Diagnostics.Stopwatch]::StartNew()
            while ($header.Count -lt 8192 -and $headerClock.ElapsedMilliseconds -lt 5000) {
                $next = $stream.ReadByte()
                if ($next -lt 0) { break }
                $header.Add([byte]$next)
                $n = $header.Count
                if ($n -ge 4 -and $header[$n-4] -eq 13 -and $header[$n-3] -eq 10 -and $header[$n-2] -eq 13 -and $header[$n-1] -eq 10) {
                    $complete = $true
                    break
                }
            }
            if (-not $complete) { continue }
            $lines = [Text.Encoding]::ASCII.GetString($header.ToArray()).Split(@("`r`n"), [StringSplitOptions]::None)
            $request = [regex]::Match($lines[0], '^(GET|HEAD) (/[^ ]*) HTTP/1\.[01]$')
            $status, $reason, $method, $target = 400, 'Bad Request', 'GET', ''
            [byte[]]$body = [Text.Encoding]::UTF8.GetBytes('Invalid sample request.')
            $extra = ''
            if ($request.Success) {
                $method, $target = $request.Groups[1].Value, $request.Groups[2].Value
                $hosts = @($lines | Where-Object { $_ -match '(?i)^Host:' })
                if ($hosts.Count -eq 1 -and $hosts[0] -match "(?i)^Host:\s*127\.0\.0\.1:$actualPort\s*$") {
                    $status, $reason = 404, 'Not Found'
                    $body = [Text.Encoding]::UTF8.GetBytes('Unknown sample resource.')
                    if ($target -ceq '/sample-episode') {
                        $status, $reason = 200, 'OK'
                        $body = [Text.Encoding]::UTF8.GetBytes('Generated offline episode. Use this URL in Download Engine, Extensions, Offline episode downloader.')
                    } elseif ($routes.ContainsKey($target)) {
                        $status, $reason = 200, 'OK'
                        $body = $routes[$target]
                        $ranges = @($lines | Where-Object { $_ -match '(?i)^Range:' })
                        if ($ranges.Count) {
                            $range = [regex]::Match($ranges[0], '(?i)^Range:\s*bytes=([0-9]{1,9})-([0-9]{1,9})\s*$')
                            if ($ranges.Count -ne 1 -or -not $range.Success -or
                                [long]$range.Groups[1].Value -gt [long]$range.Groups[2].Value -or
                                [long]$range.Groups[2].Value -ge $body.Length) {
                                $status, $reason = 416, 'Range Not Satisfiable'
                                $extra = "Content-Range: bytes */$($body.Length)`r`n"
                                $body = [byte[]]::new(0)
                            } else {
                                $first, $last, $size = [int]$range.Groups[1].Value, [int]$range.Groups[2].Value, $body.Length
                                $body = [byte[]]$body[$first..$last]
                                $status, $reason = 206, 'Partial Content'
                                $extra = "Content-Range: bytes $first-$last/$size`r`n"
                            }
                        }
                    }
                }
            }
            $response = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 $status $reason`r`nContent-Length: $($body.Length)`r`nContent-Type: application/octet-stream`r`nConnection: close`r`nCache-Control: no-store`r`nX-Content-Type-Options: nosniff`r`n$extra`r`n")
            $stream.Write($response, 0, $response.Length)
            if ($method -ne 'HEAD') {
                for ($offset = 0; $offset -lt $body.Length; $offset += 1024) {
                    $count = [Math]::Min(1024, $body.Length - $offset)
                    $stream.Write($body, $offset, $count)
                    if ($ChunkDelayMs) { Start-Sleep -Milliseconds $ChunkDelayMs }
                }
            }
            $loggedPath = if ($routes.ContainsKey($target)) { $target } else { '/unmatched' }
            Write-Output "REQUEST $method $loggedPath $status"
        } catch [IO.IOException] {
            # Pause/Stop closes the connection. Drop just that request and accept
            # the next one; socket timeouts also bound incomplete HTTP headers.
        } catch [Net.Sockets.SocketException] {
        } finally {
            $client.Dispose()
        }
    }
} finally {
    $listener.Stop()
}
