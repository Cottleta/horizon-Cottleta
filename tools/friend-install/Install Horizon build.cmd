@echo off
rem Double-click this. It runs the installer next to it and keeps the window open.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
pause
