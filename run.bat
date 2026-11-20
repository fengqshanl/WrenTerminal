@echo off
chcp 65001 >nul
cd /d "%~dp0"

rem 优先启动独立桌面应用；找不到时退回命令行 Shell。
set "WREN_DIR=%~dp0"
if exist "%~dp0target\release\wren-app.exe" set "WREN_DIR=%~dp0target\release"
if exist "%WREN_DIR%\wren-app.exe" (
  start "" "%WREN_DIR%\wren-app.exe"
  exit /b 0
)

title Wren - A lightweight Linux-style shell on Windows
"%WREN_DIR%\wren.exe"
echo.
echo [Wren exited, code=%errorlevel%]
echo Press any key to close this window...
pause >nul
