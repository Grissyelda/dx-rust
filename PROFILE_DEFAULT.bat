@echo off
rem Profiles Liberty Island with the DEFAULT settings and no mods (what a new player gets) for 45 s, with Bevy's
rem spans: prof_default.log gets frame times, the slowest systems, every span and GPU time per pass.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
if exist prof_done.txt del /f /q prof_done.txt
echo === build started %date% %time% === > build_spans.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview --features spans --target-dir target\spans >> build_spans.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build_spans.log & echo failed > prof_done.txt & exit /b 1)
echo === BUILD OK %date% %time% === >> build_spans.log
if not exist test_out mkdir test_out
rem Default settings, but VSync off so the log shows what a frame really costs (not the 60 fps cap).
(echo VSync=0)> "%~dp0test_out\default_settings.txt"
set DXVIEW_PROF=1
set DXVIEW_PROF_TIMELINE=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_NO_MODS=1
set DXVIEW_SETTINGS=%~dp0test_out\default_settings.txt
set DXVIEW_SHOT_TIMES=%~dp0test_out;44
"%~dp0target\spans\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_default.log" 2>&1
echo done > "%~dp0prof_done.txt"
