@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d C:\Users\Administrator\oi_enhancements\prisir_ime
"C:\Users\Administrator\.cargo\bin\cargo.exe" test --lib %*
echo TEST_EXITCODE=%errorlevel%
