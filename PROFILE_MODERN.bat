@echo off
rem Builds, then profiles Liberty Island for 75 s with your own settings: Modern for the first 35 s, then the
rem Classic / Modern switch flips it to Classic (on a copy of your settings: your own look is left as it was). Frame times, slowest systems and GPU time per pass go to prof.log.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
copy /y "%~dp0target\fast\settings.txt" "%~dp0test_out\modern_settings.txt" >nul
set DXVIEW_SETTINGS=%~dp0test_out\modern_settings.txt
set DXVIEW_PROF=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_REMASTER_AT=35
set DXVIEW_SHOT_TIMES=%~dp0test_out;20,50,75
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof.log" 2>&1
echo done > "%~dp0prof_done.txt"
