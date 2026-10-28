@echo off
rem Automated viewer tests: each run walks JC, optionally frobs, saves a screenshot. Output in test.log.
cd /d "%~dp0"
if not exist test_out mkdir test_out
echo === tests %date% %time% === > test_out\test.log
echo --- door_locked --- >> test_out\test.log
set DXVIEW_WALKTEST=-4712;2420;-150;-90;0.5;%~dp0test_out\test_door_locked.png;2.0
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" >> test_out\test.log 2>&1
echo --- door_slide --- >> test_out\test.log
set DXVIEW_WALKTEST=3620;-1616;-150;180;0.35;%~dp0test_out\test_door_slide.png;2.0
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" >> test_out\test.log 2>&1
echo --- push_crate --- >> test_out\test.log
set DXVIEW_WALKTEST=-3712;10640;-230;-90;2.0;%~dp0test_out\test_push_crate.png;0
"%~dp0target\fast\dxview.exe" "C:\Games\Deus Ex" >> test_out\test.log 2>&1
echo === TESTS DONE === >> test_out\test.log
