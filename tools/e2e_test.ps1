# E2E test: set test-replay mode via settings.json, launch app, screenshot during replay
$ErrorActionPreference = "Stop"
$bin = "target\debug\fps-audio-radar.exe"
if (-not (Test-Path $bin)) { Write-Error "build first"; exit 1 }

# 1. configure test mode (python writes clean UTF-8)
python tools\set_test_mode.py

# 2. launch
$env:RUST_LOG = "fps_audio_radar=debug,wgpu=warn,wgpu_core=warn,wgpu_hal=warn,winit=warn"
$proc = Start-Process -FilePath $bin -PassThru -WorkingDirectory (Get-Location) -RedirectStandardError (Join-Path (Get-Location) "app_stderr.log")
Write-Host "app started PID=$($proc.Id)"
Start-Sleep -Seconds 8

# 3. screenshot during replay
Add-Type -AssemblyName System.Drawing -ErrorAction SilentlyContinue
Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$bmp.Save((Join-Path (Get-Location) "shot_replay.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host "screenshot saved: shot_replay.png"

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Write-Host "=== app log (last 25) ==="
Get-Content app_stderr.log | Select-String -Pattern "fps_audio_radar" | Select-Object -Last 25 | ForEach-Object { $_.Line }
