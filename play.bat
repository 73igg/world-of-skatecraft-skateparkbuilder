@echo off
rem Starts Docker, the local server, and World of Skatecraft.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0play.ps1" %*
if errorlevel 1 pause
