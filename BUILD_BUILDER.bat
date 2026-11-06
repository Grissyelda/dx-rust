@echo off
cd /d "%~dp0"
rem Builds the DX RUST Release Builder (crates\dxbuilder, its own cargo workspace) as DXBuilder.exe in this folder and
rem opens it. Run this once (and again after the builder's code changes); after that, use DXBuilder.exe to make the
rem Windows, Linux / Steam Deck and macOS releases.
echo === builder build started %date% %time% === > builder_build.log
pushd crates\dxbuilder
"%USERPROFILE%\.cargo\bin\cargo.exe" build --release >> ..\..\builder_build.log 2>&1
if errorlevel 1 (popd & echo === BUILD FAILED === >> builder_build.log & exit /b 1)
popd
taskkill /im DXBuilder.exe /f >nul 2>&1
copy /y crates\dxbuilder\target\release\dxbuilder.exe DXBuilder.exe >> builder_build.log 2>&1
echo === BUILD OK %date% %time% === >> builder_build.log
start "" "%~dp0DXBuilder.exe"
