@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
dumpbin /exports C:\Users\Administrator\oi_enhancements\installer\stage\prisir_ime.dll > C:\Users\Administrator\oi_enhancements\prisir_ime\exp.txt 2>&1
