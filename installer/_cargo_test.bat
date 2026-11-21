@echo off
rem ------------------------------------------------
rem Lingxi IME test helper: cargo <subcmd> inside vcvars64.
rem Usage: _cargo_test.bat <crate_dir> <subcmd> [args...]
rem   e.g. _cargo_test.bat C:\...\prisir_ime test --release --lib
rem Keep ASCII-only (cmd reads it as GBK).
rem ------------------------------------------------
setlocal
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if errorlevel 1 (
  echo MSVC_VCVARS64_FAILED
  exit /b 1
)
if "%~2"=="" (
  echo USAGE: _cargo_test.bat ^<crate_dir^> ^<cargo subcmd^> [args...]
  exit /b 2
)
cd /d "%~1" || exit /b 3
cargo %2 %3 %4 %5 %6 %7 %8 %9
exit /b %errorlevel%
