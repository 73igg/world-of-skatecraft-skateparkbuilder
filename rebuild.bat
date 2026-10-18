@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Build the current checkout only. Initial setup must already be complete.
pushd "%~dp0"
if errorlevel 1 exit /b 1

set "SKATECRAFT_CARGO=cargo"
where cargo >nul 2>nul
if errorlevel 1 (
    if exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
        set "SKATECRAFT_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
    ) else (
        echo Rust Cargo was not found. Install Rust or open a new terminal after installing it.
        set "SKATECRAFT_BUILD_EXIT=1"
        goto finish
    )
)

if not exist ".setup\skate-camera-engine\skate\crates\skate-host\Cargo.toml" (
    echo The prepared Skate engine is missing. Complete the initial setup with setup.bat first.
    set "SKATECRAFT_BUILD_EXIT=1"
    goto finish
)

echo Recompiling World of Skatecraft from this checkout...
echo Close the game before rebuilding. Existing Cargo build files will be reused.
call "%SKATECRAFT_CARGO%" build --locked --release -p benilla --jobs 1
set "SKATECRAFT_BUILD_EXIT=%ERRORLEVEL%"
if not "%SKATECRAFT_BUILD_EXIT%"=="0" (
    echo.
    echo Rebuild failed. See the compiler message above.
    goto finish
)

echo.
echo Rebuild complete: "%CD%\target\release\benilla.exe"
echo Start the game with play.bat when ready.

:finish
popd
if /i not "%~1"=="--no-pause" pause
exit /b %SKATECRAFT_BUILD_EXIT%
