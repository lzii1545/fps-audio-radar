# E2E drag-position test with logs
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M6 {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public struct RECT { public int L, T, R, B; }
    delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
    public static void Click(int x, int y) {
        SetCursorPos(x, y); System.Threading.Thread.Sleep(100);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(500);
    }
    public static void Drag(int x0, int y0, int x1, int y1) {
        SetCursorPos(x0, y0); System.Threading.Thread.Sleep(150);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(100);
        for (int i = 1; i <= 10; i++) {
            SetCursorPos(x0 + (x1-x0)*i/10, y0 + (y1-y0)*i/10);
            System.Threading.Thread.Sleep(40);
        }
        System.Threading.Thread.Sleep(100);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(600);
    }
    public static RECT GetWin(uint pid) {
        RECT r = new RECT();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                RECT rr; GetWindowRect(h, out rr);
                if (rr.R - rr.L > 500) r = rr;
            }
            return true;
        }, IntPtr.Zero);
        return r;
    }
}
"@
function Shot($name) {
    Add-Type -AssemblyName System.Drawing -ErrorAction SilentlyContinue
    Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save((Join-Path (Get-Location) $name), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

$env:RUST_LOG = "fps_audio_radar=debug"
$proc = Start-Process -FilePath "target\release\fps-audio-radar.exe" -PassThru -WorkingDirectory (Get-Location) -RedirectStandardError "edit_log.txt"
Start-Sleep -Seconds 6
$r = [M6]::GetWin($proc.Id)
"window: ($($r.L),$($r.T))"
# 1. 点击导航「位置」（第三个导航项）
[M6]::Click($r.L + 83, $r.T + 187)
Start-Sleep -Seconds 1
Shot("edit0.png")
# 2. 用 python 找内容区的主按钮（亮蓝块）并点击
$btn = & "C:\Users\ASUS\.dsh\dsh-runtimes\dsh-primary-runtime\dependencies\python\python.exe" -c "
from PIL import Image
im = Image.open('edit0.png').convert('RGB')
px = im.load()
# 内容区 x > L+166
best = None
for y in range(0, 1080, 3):
    for x in range(0, 1920, 3):
        r,g,b = px[x,y]
        if abs(r-0)<=15 and abs(g-166)<=15 and abs(b-255)<=18:
            # 找按钮块的左上角
            if best is None or y > best[1]:
                best = (x,y)
print(best[0], best[1])
"
$btn
$bx, $by = $btn -split ' '
[M6]::Click([int]$bx + 60, [int]$by + 16)
Start-Sleep -Seconds 2
Shot("edit1.png")
# 3. 拖拽雷达中心手柄（屏幕中心）到右下
[M6]::Drag(960, 540, 1120, 660)
Start-Sleep -Seconds 1
# 4. 点右上角绿色对勾
[M6]::Click(1864, 56)
Start-Sleep -Seconds 1
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
"=== overlay log ==="
Get-Content edit_log.txt | Select-String -Pattern "overlay" | Select-Object -Last 15 | ForEach-Object { $_.Line }
"=== settings after ==="
$s = Get-Content (Join-Path $env:APPDATA "FPSAudioRadar\settings.json") -Raw -Encoding UTF8 | ConvertFrom-Json
"offset_x=$($s.offset_x) offset_y=$($s.offset_y)"