@echo off
rem Builds, then profiles Liberty Island twice for 40 s each with a copy of your settings (VSync off):
rem   prof_opt_a.log - as shipped
rem   prof_opt_b.log - Update without Bevy's automatic sync points (DXVIEW_NO_AUTO_SYNC, a measurement only)
rem Each log has frame times, every system's time, the schedule report (systems and sync points) and draw counts.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
if not exist "%~dp0test_out" mkdir "%~dp0test_out"
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
findstr /v /b /c:"VSync=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\opt_settings.txt"
(echo VSync=0)>> "%~dp0test_out\opt_settings.txt"
set DXVIEW_SETTINGS=%~dp0test_out\opt_settings.txt
set DXVIEW_PROF=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_SHOT_TIMES=%~dp0test_out;40
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_opt_a.log" 2>&1
set DXVIEW_NO_AUTO_SYNC=1
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_opt_b.log" 2>&1
echo done > "%~dp0prof_opt_done.txt"
