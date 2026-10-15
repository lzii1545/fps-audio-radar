# Smoke test: launch app -> play test sound -> screenshot
$ErrorActionPreference = "Stop"
$bin = "target\debug\fps-audio-radar.exe"
if (-not (Test-Path $bin)) { Write-Error "build first"; exit 1 }

$proc = Start-Process -FilePath $bin -PassThru -WorkingDirectory (Get-Location)
Write-Host "app started PID=$($proc.Id)"
Start-Sleep -Seconds 6

Add-Type -AssemblyName System.Windows.Forms
$player = New-Object System.Media.SoundPlayer((Join-Path (Get-Location) "test_sound.wav"))
$player.PlayLooping()
Write-Host "test sound playing..."
Start-Sleep -Seconds 8

Add-Type -AssemblyName System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$bmp.Save((Join-Path (Get-Location) "shot.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host "screenshot saved: shot.png"

$player.Stop(); $player.Dispose()
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Write-Host "test done, app closed"
