@echo off
cd /d "C:\Users\Administrator\oi_enhancements\prisir_ime"
C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe check --release 2>&1
pause
