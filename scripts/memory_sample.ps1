param([Parameter(Mandatory = $true)][int]$ProcessId)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MemorySoakNative {
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    public static bool IsForeground(int pid) {
        uint owner; GetWindowThreadProcessId(GetForegroundWindow(), out owner);
        return owner == pid;
    }
}
'@
while ($true) {
    $observed = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $observed) { break }
    try {
        $observed.Refresh()
        [ordered]@{
            at = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            pid = $ProcessId
            privateBytes = $observed.PrivateMemorySize64
            workingSet = $observed.WorkingSet64
            handles = $observed.HandleCount
            threads = $observed.Threads.Count
            gdi = [MemorySoakNative]::GetGuiResources($observed.Handle, 0)
            user = [MemorySoakNative]::GetGuiResources($observed.Handle, 1)
            cpuSeconds = $observed.TotalProcessorTime.TotalSeconds
            foreground = [MemorySoakNative]::IsForeground($ProcessId)
        } | ConvertTo-Json -Compress
    } finally { $observed.Dispose() }
    Start-Sleep -Seconds 1
}
