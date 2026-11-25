@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d C:\Users\Administrator\oi_enhancements\prisiragent_ime
"C:\Users\Administrator\.cargo\bin\cargo.exe" build --release 2>&1
echo BUILD_EXITCODE=%errorlevel%
