@echo off
chcp 65001 >nul
title Wren - A lightweight Linux-style shell on Windows
cd /d "D:\code\linuxTerminal"
"D:\code\linuxTerminal\target\release\wren.exe"
echo.
echo [Wren exited, code=%errorlevel%]
echo Press any key to close this window...
pause >nul
