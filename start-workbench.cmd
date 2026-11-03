@echo off
where pwsh >nul 2>nul
if errorlevel 1 (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-workbench.ps1" %*
) else (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start-workbench.ps1" %*
)
if errorlevel 1 pause
