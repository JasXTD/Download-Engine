# Run in an x64 Visual Studio developer PowerShell. Only the encoder and its
# upstream command-line frontend are built. No network or external codec SDKs.
param([Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64') { throw 'Use the x64 Native Tools developer shell.' }
$archive = Join-Path $PSScriptRoot 'lame-4.0.tar.gz'
if ((Get-FileHash -LiteralPath $archive).Hash -ine '3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb') {
    throw 'LAME source checksum mismatch'
}
$destination = [IO.Path]::GetFullPath($Output)
$temporary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$work = [IO.Directory]::CreateTempSubdirectory('de-lame-')
try {
    tar -xzf $archive -C $work.FullName
    if ($LASTEXITCODE) { throw 'LAME source extraction failed' }
    Push-Location -LiteralPath (Join-Path $work.FullName 'lame-4.0')
    try {
        Copy-Item -LiteralPath 'configMS.h' -Destination 'config.h'
        $library = @('VbrTag','bitstream','encoder','fft','gain_analysis','id3tag','lame','newmdct','presets','psymodel','quantize','quantize_pvt','reservoir','set_get','tables','takehiro','util','vbrquantize','version','mpglib_interface') | ForEach-Object { "libmp3lame/$_.c" }
        $frontend = @('main','lame_main','brhist','console','get_audio','lametime','parse','timestatus') | ForEach-Object { "frontend/$_.c" }
        & cl.exe /nologo /O2 /MT /GS /guard:cf /utf-8 /DHAVE_CONFIG_H /I. /Iinclude /Ilibmp3lame @library libmp3lame/vector/xmm_quantize_sub.c @frontend /Felame.exe /link /Brepro /DYNAMICBASE /NXCOMPAT /guard:cf
        if ($LASTEXITCODE) { throw 'LAME compilation failed' }
        Copy-Item -LiteralPath 'lame.exe' -Destination $destination
    } finally { Pop-Location }
} finally {
    $resolved = (Resolve-Path -LiteralPath $work.FullName).Path
    if ([IO.Path]::GetDirectoryName($resolved) -ne $temporary.TrimEnd('\') -or $work.Name -notmatch '^de-lame-[a-zA-Z0-9._-]+$' -or (Get-Item -LiteralPath $resolved).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Unexpected build directory during cleanup'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
