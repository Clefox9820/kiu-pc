param(
    # the test game's process id; only its window is driven or captured
    [Parameter(Mandatory)][int]$ProcessId,
    # steps separated by ';':
    #   key:<Keys name or 0xVK> [ms]   holds a key (default 120 ms)
    #   click:<x>,<y>                  touches at window client pixels
    #   mouse:<dx>,<dy>,<count>        real relative mouse motion, for mouse aim
    #   shot:<file.png>                captures the window's client area
    #   wait:<ms>
    [Parameter(Mandatory)][string]$Steps
)
# Drives a test game window for unattended checks. Keys and clicks are
# posted to the window, so they work without the focus. Example:
#   scripts\drive-window.ps1 -ProcessId 1234 -Steps "click:200,380;wait:3000;shot:C:\tmp\menu.png"
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Drv {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f);
}
"@
$h = (Get-Process -Id $ProcessId -ErrorAction Stop).MainWindowHandle
if ($h -eq [IntPtr]::Zero) { throw "no window for $ProcessId" }

function Key([uint32]$vk, [int]$ms) {
    $scan = [Drv]::MapVirtualKey($vk, 0)
    [void][Drv]::PostMessage($h, 0x100, [IntPtr]$vk, [IntPtr](1 -bor ($scan -shl 16)))
    Start-Sleep -Milliseconds $ms
    [void][Drv]::PostMessage($h, 0x101, [IntPtr]$vk, [IntPtr]([int64]1 -bor ($scan -shl 16) -bor (3L -shl 30)))
}

function Click([int]$x, [int]$y) {
    $at = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
    [void][Drv]::PostMessage($h, 0x200, [IntPtr]0, $at)
    Start-Sleep -Milliseconds 50
    [void][Drv]::PostMessage($h, 0x201, [IntPtr]1, $at)
    Start-Sleep -Milliseconds 120
    [void][Drv]::PostMessage($h, 0x202, [IntPtr]0, $at)
}

function Shot([string]$out) {
    # with the display asleep nothing is composed and every shot comes out
    # white; this resets the display's idle timer so it turns on
    [void][Drv]::SetThreadExecutionState(3)
    if ([Drv]::IsIconic($h)) { [void][Drv]::ShowWindow($h, 9) }
    [void][Drv]::SetForegroundWindow($h)
    Start-Sleep -Milliseconds 300
    $r = New-Object Drv+RECT
    [void][Drv]::GetClientRect($h, [ref]$r)
    $o = New-Object Drv+POINT
    [void][Drv]::ClientToScreen($h, [ref]$o)
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    if ($w -lt 50 -or $hh -lt 50) { throw "window client area is only ${w}x${hh}" }
    # PrintWindow cannot read a Vulkan swapchain, so the client area is copied
    # from the screen, and only when nothing else can end up in the picture:
    # it must fit the screen and every sampled point must be this window
    $area = [System.Windows.Forms.Screen]::FromHandle($h).WorkingArea
    if ($o.X -lt $area.Left -or $o.Y -lt $area.Top -or $o.X + $w -gt $area.Right -or $o.Y + $hh -gt $area.Bottom) {
        throw "the window goes past the screen's working area; not capturing"
    }
    foreach ($fx in 0.02, 0.25, 0.5, 0.75, 0.98) {
        foreach ($fy in 0.02, 0.25, 0.5, 0.75, 0.98) {
            $pt = New-Object Drv+POINT
            $pt.X = $o.X + [int]($w * $fx); $pt.Y = $o.Y + [int]($hh * $fy)
            if ([Drv]::GetAncestor([Drv]::WindowFromPoint($pt), 2) -ne $h) {
                throw "the window is covered at $($pt.X),$($pt.Y); not capturing"
            }
        }
    }
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($o.X, $o.Y, 0, 0, $bmp.Size)
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    "shot $out ${w}x${hh}"
}

foreach ($step in $Steps -split ';') {
    $kind, $arg = $step -split ':', 2
    switch ($kind) {
        'key' {
            $name, $ms = $arg -split ' '
            $vk = if ($name -match '^0x') { [Convert]::ToUInt32($name, 16) } else { [uint32][System.Windows.Forms.Keys]$name }
            Key $vk ($(if ($ms) { [int]$ms } else { 120 }))
        }
        'click' { $x, $y = $arg -split ','; Click ([int]$x) ([int]$y) }
        # raw input needs the window in front, which a shot step puts there
        'mouse' {
            $dx, $dy, $n = $arg -split ','
            for ($i = 0; $i -lt [int]$n; $i++) { [Drv]::mouse_event(1, [int]$dx, [int]$dy, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 16 }
        }
        'shot' { Shot $arg }
        'wait' { Start-Sleep -Milliseconds ([int]$arg) }
        default { throw "unknown step '$step'" }
    }
}
