@echo off
rem Builds with Bevy's spans on (target\spans), then profiles Liberty Island for 45 s with your own settings (VSync off) and exits:
rem prof_spans3.log gets the usual tables, exclusive time per thread kind, and two whole frames' timelines.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build_spans.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview --features spans --target-dir target\spans >> build_spans.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build_spans.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build_spans.log
copy /y "%~dp0target\fast\settings.txt" "%~dp0target\spans\fast\settings.txt" >nul 2>&1
copy /y "%~dp0target\fast\launcher.txt" "%~dp0target\spans\fast\launcher.txt" >nul 2>&1
findstr /v /b /c:"VSync=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\spans_settings.txt"
(echo VSync=0)>> "%~dp0test_out\spans_settings.txt"
set DXVIEW_SETTINGS=%~dp0test_out\spans_settings.txt
set DXVIEW_PROF=1
set DXVIEW_PROF_TIMELINE=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_SHOT_TIMES=%~dp0test_out;44
"%~dp0target\spans\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_spans3.log" 2>&1
echo done > "%~dp0prof_spans3_done.txt"
