@echo off
rem ------------------------------------------------
rem Lingxi IME build helper: run cargo build inside vcvars64.
rem Usage: _cargo_release.bat <crate_dir> [cargo args...]
rem         _cargo_release.bat --where-link
rem Why: Git Bash -> cmd nested quotes get eaten; this .bat is the bridge.
rem Keep this file ASCII-only (cmd reads it as GBK).
rem ------------------------------------------------
setlocal
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if errorlevel 1 (
  echo MSVC_VCVARS64_FAILED
  exit /b 1
)
if "%~1"=="" (
  echo USAGE: _cargo_release.bat ^<crate_dir^> [cargo args...]
  exit /b 2
)
if "%~1"=="--where-link" (
  where link
  exit /b %errorlevel%
)
cd /d "%~1" || exit /b 3
cargo build --release %2 %3 %4 %5 %6 %7 %8
exit /b %errorlevel%
