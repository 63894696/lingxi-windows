@echo off
rem Lingxi IME build helper: run cargo build inside vcvars64.
rem Usage: _cargo_release.bat <crate_dir> [cargo args...]
rem         _cargo_release.bat --where-link
rem Keep ASCII-only. After shift, percent-star is NOT updated; use percent1..9.
setlocal EnableExtensions
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if errorlevel 1 (
  echo MSVC_VCVARS64_FAILED
  exit /b 1
)
if "%~1"=="" (
  echo USAGE: _cargo_release.bat crate_dir [cargo args...]
  exit /b 2
)
if "%~1"=="--where-link" (
  where link
  exit /b %errorlevel%
)
set "CRATE_DIR=%~1"
shift /1
cd /d "%CRATE_DIR%" || (
  echo CARGO_CD_FAILED %CRATE_DIR%
  exit /b 3
)
echo [cargo-bridge] cd=%CD%
echo [cargo-bridge] args=%1 %2 %3 %4 %5 %6 %7 %8 %9
cargo build --release %1 %2 %3 %4 %5 %6 %7 %8 %9
set "ERR=%ERRORLEVEL%"
if not "%ERR%"=="0" (
  echo CARGO_BUILD_FAILED %ERR%
  exit /b %ERR%
)
echo [cargo-bridge] ok
exit /b 0