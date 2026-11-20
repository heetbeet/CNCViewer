@echo off
cd /d "%~dp0"
if exist "output\desktop\CNC Viewer.exe" (
  start "" "output\desktop\CNC Viewer.exe" %*
  exit /b
)
if exist "target\release\cnc-viewer.exe" (
  start "" "target\release\cnc-viewer.exe" %*
  exit /b
)
echo Build CNC Viewer with powershell -File tools\build.ps1
pause
