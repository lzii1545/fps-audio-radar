# Diagnostic: quiet shot vs sound shot + window enumeration
$ErrorActionPreference = "Stop"
$bin = "target\debug\fps-audio-radar.exe"
if (-not (Test-Path $bin)) { Write-Error "build first"; exit 1 }

$proc = Start-Process -FilePath $bin -PassThru -WorkingDirectory (Get-Location)
Write-Host "app started PID=$($proc.Id)"
Start-Sleep -Seconds 7

# enumerate top-level windows with rects
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Collections.Generic;
public class WinEnum {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    public struct RECT { public int L, T, R, B; }
    delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
    public static List<string> List() {
        var out_ = new List<string>();
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(256);
            GetWindowTextW(h, sb, 256);
            RECT r;
            GetWindowRect(h, out r);
            var t = sb.ToString();
            if (t.Length > 0) out_.Add(string.Format("{0} | ({1},{2})-({3},{4})", t, r.L, r.T, r.R, r.B));
            return true;
        }, IntPtr.Zero);
        return out_;
    }
}
"@
[WinEnum]::List() | ForEach-Object { Write-Host "WIN: $_" }

function Shot($name) {
    Add-Type -AssemblyName System.Drawing -ErrorAction SilentlyContinue
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save((Join-Path (Get-Location) $name), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "saved $name"
}

Shot("shot_quiet.png")
Start-Sleep -Seconds 2

Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
$player = New-Object System.Media.SoundPlayer((Join-Path (Get-Location) "test_sound.wav"))
$player.PlayLooping()
Start-Sleep -Seconds 5
Shot("shot_sound.png")

$player.Stop(); $player.Dispose()
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Write-Host "done"
