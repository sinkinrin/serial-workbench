@echo off
pwsh -NoProfile -File "%~dp0scripts\start-workbench.ps1" %*
if errorlevel 1 pause
