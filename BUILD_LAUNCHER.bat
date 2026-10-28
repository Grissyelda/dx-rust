@echo off
cd /d "%~dp0"
rem Builds the launcher (its own cargo workspace) and puts it next to dxview.exe, where it expects to live.
taskkill /im dxlauncher.exe /f >nul 2>&1
echo === launcher build started %date% %time% === > launcher_build.log
pushd crates\dxlauncher
"%USERPROFILE%\.cargo\bin\cargo.exe" build --release >> ..\..\launcher_build.log 2>&1
if errorlevel 1 (popd & echo === BUILD FAILED === >> launcher_build.log & exit /b 1)
popd
if not exist target\fast mkdir target\fast
copy /y crates\dxlauncher\target\release\dxlauncher.exe target\fast\dxlauncher.exe >> launcher_build.log 2>&1
echo === BUILD OK %date% %time% === >> launcher_build.log
