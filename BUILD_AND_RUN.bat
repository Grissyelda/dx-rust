@echo off
rem Builds the viewer from source on this PC and starts it. Output goes to build.log.
cd /d "%~dp0"
rem A running viewer locks dxview.exe; close it first.
taskkill /im dxview.exe /f >nul 2>&1
rem A running exe cannot be deleted but can be renamed, so move any locked copy aside.
del /f /q target\fast\dxview.old*.exe >nul 2>&1
if exist target\fast\dxview.exe move /y target\fast\dxview.exe target\fast\dxview.old%RANDOM%.exe >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (
  echo === BUILD FAILED === >> build.log
  echo Build failed, see build.log
  exit /b 1
)
echo === BUILD OK %date% %time% === >> build.log
start "" "%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex"
