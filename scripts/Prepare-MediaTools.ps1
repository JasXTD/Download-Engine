[CmdletBinding()]
param([switch]$TestTools)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Media-Bundle.ps1')
Assert-MediaBundle -Root $workspaceRoot -TestTools:$TestTools
Write-Output 'Pinned FFmpeg binary, corresponding sources, build recipe and notices verified.'
