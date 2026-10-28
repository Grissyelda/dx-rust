@echo off
rem Starts the most recent build with its console output in run.log (closes a running copy first).
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
start "" /b cmd /c ""%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" > "%~dp0run.log" 2>&1"
