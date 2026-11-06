@echo off
cd /d "%~dp0"
rem Builds DX RUST Online Services (the lobby server and its controller, its own cargo workspace), puts
rem dxlobby.exe in the online folder and opens the controller. The controller starts the server in the background;
rem closing the controller leaves the server running.
if not exist online mkdir online
echo === lobby build started %date% %time% === > lobby_build.log
pushd crates\dxlobby
"%USERPROFILE%\.cargo\bin\cargo.exe" build --release >> ..\..\lobby_build.log 2>&1
if errorlevel 1 (popd & echo === BUILD FAILED === >> lobby_build.log & exit /b 1)
popd
taskkill /im dxlobby.exe /f >nul 2>&1
copy /y crates\dxlobby\target\release\dxlobby.exe online\dxlobby.exe >> lobby_build.log 2>&1
echo === BUILD OK %date% %time% === >> lobby_build.log
start "" "%~dp0online\dxlobby.exe"
