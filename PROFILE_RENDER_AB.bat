@echo off
rem Builds, then profiles Liberty Island three times (70 s each, your settings with VSync and the frame cap off;
rem your look for 35 s, then the other look): as now, with the render world's schedule run in order on the render
rem thread, with 6 worker threads, and on DirectX 12 instead of Vulkan. Logs: prof_ab_base.log, prof_ab_strender.log,
rem prof_ab_t6.log, prof_ab_dx12.log.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
if not exist test_out mkdir test_out
findstr /v /b /c:"VSync=" /c:"FPSLimit=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\ab_settings.txt"
(echo VSync=0)>> "%~dp0test_out\ab_settings.txt"
(echo FPSLimit=0)>> "%~dp0test_out\ab_settings.txt"
set DXVIEW_PROF=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_REMASTER_AT=35
set DXVIEW_SHOT_TIMES=%~dp0test_out;30,70
rem Each run gets a fresh copy (the Classic / Modern switch writes the settings file).
copy /y "%~dp0test_out\ab_settings.txt" "%~dp0test_out\ab_run.txt" >nul
set DXVIEW_SETTINGS=%~dp0test_out\ab_run.txt
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_ab_base.log" 2>&1
copy /y "%~dp0test_out\ab_settings.txt" "%~dp0test_out\ab_run.txt" >nul
set DXVIEW_ST_RENDER=1
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_ab_strender.log" 2>&1
set DXVIEW_ST_RENDER=
copy /y "%~dp0test_out\ab_settings.txt" "%~dp0test_out\ab_run.txt" >nul
set DXVIEW_THREADS=6
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_ab_t6.log" 2>&1
set DXVIEW_THREADS=
copy /y "%~dp0test_out\ab_settings.txt" "%~dp0test_out\ab_run.txt" >nul
set WGPU_BACKEND=dx12
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_ab_dx12.log" 2>&1
set WGPU_BACKEND=
echo done > "%~dp0prof_ab_done.txt"
