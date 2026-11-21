@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 (
  echo VCVARS_FAILED
  exit /b 1
)
cd /d C:\Users\Administrator\oi_enhancements\prisir_ime
echo === cl.exe ===
where cl
echo === link.exe ===
where link
echo === cargo ===
"C:\Users\Administrator\.cargo\bin\cargo.exe" --version
echo === BUILD START ===
"C:\Users\Administrator\.cargo\bin\cargo.exe" build --release
echo BUILD_EXITCODE=%errorlevel%
