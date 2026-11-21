@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
dumpbin /exports C:\Users\Administrator\oi_enhancements\prisir_ime\target\release\prisir_ime.dll 2>&1 | findstr /C:"prisir_ime_"
