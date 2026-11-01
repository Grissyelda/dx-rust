@echo off
rem Builds, then profiles Liberty Island for 70 s with your own settings but VSync off: your look for 35 s, then the
rem other look (Classic / Modern switch). Frame times, draw calls per render phase and GPU pass times go to prof_gpu.log.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
findstr /v /b /c:"VSync=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\gpu_settings.txt"
(echo VSync=0)>> "%~dp0test_out\gpu_settings.txt"
set DXVIEW_SETTINGS=%~dp0test_out\gpu_settings.txt
set DXVIEW_PROF=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_REMASTER_AT=35
set DXVIEW_SHOT_TIMES=%~dp0test_out;30,70
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_gpu.log" 2>&1
echo done > "%~dp0prof_gpu_done.txt"
