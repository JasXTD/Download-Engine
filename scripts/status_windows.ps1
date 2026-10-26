param([Parameter(Mandatory)][int]$WindowProcess, [ValidateSet('List','CloseStatus','CloseApp')][string]$Action = 'List')
$ErrorActionPreference = 'Stop'
# Inspect or close only windows belonging to this explicitly supplied test PID.
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class StatusWindows {
  delegate bool EnumProc(IntPtr handle, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr data);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr handle, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr handle, StringBuilder text, int count);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr handle);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr handle, uint message, IntPtr wparam, IntPtr lparam);
  public sealed class Window { public long Handle; public string Title; }
  public static Window[] Find(int pid, string action) {
    var windows = new List<Window>();
    EnumWindows((handle, data) => {
      uint owner; GetWindowThreadProcessId(handle, out owner);
      if (owner != pid || !IsWindowVisible(handle)) return true;
      var title = new StringBuilder(1024); GetWindowText(handle, title, title.Capacity);
      var text = title.ToString();
      // Status windows can show progress and the filename, or Download complete.
      bool status = text.StartsWith("Download status") || text.EndsWith("\u2014 Download Engine");
      if (text != "Download Engine" && !status) return true;
      windows.Add(new Window {Handle=handle.ToInt64(), Title=text});
      if ((action=="CloseApp" && text=="Download Engine") || (action=="CloseStatus" && status))
        if (!PostMessage(handle, 0x0010, IntPtr.Zero, IntPtr.Zero)) throw new Exception("Could not close the test window");
      return true;
    }, IntPtr.Zero);
    return windows.ToArray();
  }
}
'@
ConvertTo-Json -InputObject @([StatusWindows]::Find($WindowProcess, $Action)) -Compress
