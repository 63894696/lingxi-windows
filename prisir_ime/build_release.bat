@echo off
cd /d "C:\Users\Administrator\oi_enhancements\prisir_ime"
echo 正在编译 prisir_ime...
C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe build --release 2>&1
echo 编译完成
