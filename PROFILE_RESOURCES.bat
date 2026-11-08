@echo off
rem Builds, then profiles one session with your settings (a copy, with VSync and the frame cap off): Liberty Island
rem for 70 s, then the boat to Battery Park and 60 s there. Every 5 s the log gets frame times, CPU (cores used),
rem memory (resident / committed), GPU memory (the driver's allocator and Bevy's textures) and what the CPU side keeps
rem of images and meshes. Log: prof_res.log. Send it back when it's done (prof_res_done.txt appears).
cd /d "%~dp0"
taskkill /im dxview.exe /f >nul 2>&1
echo === build started %date% %time% === > build.log
"%USERPROFILE%\.cargo\bin\cargo.exe" build --profile fast -p dxview >> build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> build.log & exit /b 1)
echo === BUILD OK %date% %time% === >> build.log
if not exist test_out mkdir test_out
findstr /v /b /c:"VSync=" /c:"FPSLimit=" "%~dp0target\fast\settings.txt" > "%~dp0test_out\res_settings.txt"
(echo VSync=0)>> "%~dp0test_out\res_settings.txt"
(echo FPSLimit=0)>> "%~dp0test_out\res_settings.txt"
set DXVIEW_PROF=1
set DXVIEW_SKIP_MENU=1
set DXVIEW_SETTINGS=%~dp0test_out\res_settings.txt
rem At 70 s: onto the dock's MapExit (the boat to Battery Park). Screenshots at 60 and 150 s; the run ends after the last.
set DXVIEW_GOTO=70;-4779,10346,-200;tp
set DXVIEW_SHOT_TIMES=%~dp0test_out;60,150
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" 01_NYC_UNATCOIsland.dx > "%~dp0prof_res.log" 2>&1
echo done > "%~dp0prof_res_done.txt"
