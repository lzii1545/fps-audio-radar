# Diagnostic: run app with filtered stderr log, play sound, dump log
$ErrorActionPreference = "Stop"
$bin = "target\debug\fps-audio-radar.exe"
if (-not (Test-Path $bin)) { Write-Error "build first"; exit 1 }

$env:RUST_LOG = "fps_audio_radar=debug,wgpu=warn,wgpu_core=warn,wgpu_hal=warn,winit=warn"
$proc = Start-Process -FilePath $bin -PassThru -WorkingDirectory (Get-Location) -RedirectStandardError (Join-Path (Get-Location) "app_stderr.log")
Write-Host "app started PID=$($proc.Id)"
Start-Sleep -Seconds 7

Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
$player = New-Object System.Media.SoundPlayer((Join-Path (Get-Location) "test_sound.wav"))
$player.PlayLooping()
Start-Sleep -Seconds 6
$player.Stop(); $player.Dispose()

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Write-Host "=== app log ==="
Get-Content app_stderr.log | Select-String -Pattern "fps_audio_radar" | Select-Object -Last 30 | ForEach-Object { $_.Line }
