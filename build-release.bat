@echo off
setlocal
cd /d %~dp0
if not exist build mkdir build

echo === Building CPU edition ===
cargo build --release --bin thocoin-gui || exit /b 1
copy /y target\release\thocoin-gui.exe build\thocoin-gui-cpu.exe || exit /b 1

echo === Building GPU edition ===
cargo build --release --features gpu --bin thocoin-gui || exit /b 1
copy /y target\release\thocoin-gui.exe build\thocoin-gui-gpu.exe || exit /b 1

echo === Done. Now build thocoin-installer.iss in Inno Setup (F9) ===
