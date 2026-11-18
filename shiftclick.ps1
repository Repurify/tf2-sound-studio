Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Inp {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
}
"@
[Inp]::SetCursorPos([int]$args[0], [int]$args[1])
Start-Sleep -Milliseconds 200
[Inp]::keybd_event(0x10, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 150
[Inp]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
[Inp]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 150
[Inp]::keybd_event(0x10, 0, 2, [IntPtr]::Zero)
Write-Output "shift-click sent"
