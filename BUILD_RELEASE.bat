@echo off
cd /d "%~dp0"
rem Builds a shareable copy in dist\: dxview.exe + dxlauncher.exe, full release optimisation, the C runtime linked
rem in (no Visual C++ Redistributable needed on the other PC). Own build folders, so the quick builds keep their
rem cache. The first run compiles everything (several minutes). No game files are copied: the player picks their
rem own Deus Ex folder in the launcher.
echo === release build started %date% %time% === > release_build.log
set RUSTFLAGS=-C target-feature=+crt-static
set CARGO_TARGET_DIR=%~dp0target\dist-build
"%USERPROFILE%\.cargo\bin\cargo.exe" build --release -p dxview >> release_build.log 2>&1
if errorlevel 1 (echo === BUILD FAILED === >> release_build.log & exit /b 1)
set CARGO_TARGET_DIR=%~dp0target\dist-launcher
pushd crates\dxlauncher
"%USERPROFILE%\.cargo\bin\cargo.exe" build --release >> ..\..\release_build.log 2>&1
if errorlevel 1 (popd & echo === BUILD FAILED === >> release_build.log & exit /b 1)
popd
if not exist dist mkdir dist
copy /y target\dist-build\release\dxview.exe dist\dxview.exe >> release_build.log 2>&1
copy /y target\dist-launcher\release\dxlauncher.exe dist\dxlauncher.exe >> release_build.log 2>&1
copy /y LICENSE dist\LICENSE.txt >> release_build.log 2>&1
copy /y README.md dist\README.md >> release_build.log 2>&1
copy /y THIRD_PARTY.md dist\THIRD_PARTY.md >> release_build.log 2>&1
rem The release download: dist\dxrust-windows.zip (the two exes and the notices, nothing of the game's).
if exist dist\dxrust-windows.zip del /f /q dist\dxrust-windows.zip
powershell -NoProfile -Command "Compress-Archive -Path dist\dxview.exe,dist\dxlauncher.exe,dist\LICENSE.txt,dist\README.md,dist\THIRD_PARTY.md -DestinationPath dist\dxrust-windows.zip" >> release_build.log 2>&1
echo === BUILD OK %date% %time% === >> release_build.log
