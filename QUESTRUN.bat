@echo off
rem The quest rig: loads every map of the game in mission order, lets each run 10 s with the earlier missions'
rem story flags set and god mode on, then writes questrun.log (mission script, conversations, pawns, load time,
rem the goals and flags each map set by itself, and anything that went wrong). Your own settings and saves are
rem left alone. Usage: QUESTRUN.bat [map,map,...]   (default: every map)
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
if not exist test_out mkdir test_out
rem A copy of the settings (the run must never write the player's own file), VSync off so it goes faster.
if exist settings.txt (copy /y settings.txt test_out\quest_settings.txt >nul) else (type nul > test_out\quest_settings.txt)
(echo VSync=0)>> test_out\quest_settings.txt
set DXVIEW_SETTINGS=%~dp0test_out\quest_settings.txt
set DXVIEW_SAVE_DIR=%~dp0test_out\quest_saves
set DXVIEW_SKIP_MENU=1
set DXVIEW_GOD=1
set DXVIEW_QUESTRUN_FLAGS=1
set DXVIEW_QUESTRUN_SECS=10
if "%~1"=="" (set DXVIEW_QUESTRUN=all) else (set DXVIEW_QUESTRUN=%~1)
"%~dp0target\fast\dxview.exe" > "%~dp0questrun_stdout.log" 2>&1
echo Quest run finished: see questrun.log beside dxview.exe (or in the DX RUST data folder).
