@echo off
REM ThoCoin seed DuckDNS auto-updater. Chay tren may seed (may nha bat 24/7).
REM Thay YOUR_TOKEN bang token tu trang duckdns.org.
set DOMAIN=thocoin
set TOKEN=cf3de08f-6f73-4fdf-abf0-4b8c16bfe875

:loop
curl -s "https://www.duckdns.org/update?domains=%DOMAIN%&token=%TOKEN%&ip=" >nul
timeout /t 300 /nobreak >nul
goto loop
