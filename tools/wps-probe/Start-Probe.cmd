@echo off
chcp 65001 >nul
set "CELESVAULT_PROBE_DIR=%~dp0"
powershell.exe -NoProfile -Command "try { $p = Start-Process -FilePath (Join-Path $env:CELESVAULT_PROBE_DIR 'celesvault-probe.exe') -WorkingDirectory $env:CELESVAULT_PROBE_DIR -Verb RunAs -Wait -PassThru -ErrorAction Stop; Write-Host ('Probe exit code: ' + $p.ExitCode); if ($p.ExitCode -ne 0) { Write-Host 'Capture failed or was incomplete. Check logs and error messages.' } } catch { Write-Host $_.Exception.Message; exit 2 }"
echo.
echo Test finished. Reports are in the logs folder next to this launcher.
echo Please inspect summary.md before sharing the run folder. No source files are needed.
pause
