@echo off
rem Builds, then measures what each Modern feature costs on this PC: 20 s warm-up with your settings (VSync off), then
rem each feature switched off for 8 s in turn. Results ("PROF FEATURE ...") go to prof_features.log; the game exits itself.
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
if not exist "%~dp0test_out" mkdir "%~dp0test_out"
findstr /v /b /c:"VSync=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\features_settings.txt"
(echo VSync=0)>> "%~dp0test_out\features_settings.txt"
set DXVIEW_SETTINGS=%~dp0test_out\features_settings.txt
set DXVIEW_SKIP_MENU=1
set DXVIEW_PROF=1
set DXVIEW_PROF_FEATURES=1
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_features.log" 2>&1
echo done > "%~dp0prof_features_done.txt"
