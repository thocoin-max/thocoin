@echo off
setlocal
cd /d %~dp0

echo === Bump version (publish a new release) ===
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0bump-version.ps1" || exit /b 1

call build-release.bat || exit /b 1

echo === Publish version.json to explorer ===
cd /d D:\thocoin-explorer\explorer
docker compose up -d nginx

echo === Released. Users will now see the update banner. ===
