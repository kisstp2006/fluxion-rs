@echo off
REM Fluxion-RS build helper (Windows)
REM Usage:
REM   build.bat              - release build of the whole workspace
REM   build.bat debug        - debug build of the whole workspace
REM   build.bat run          - release build + run the editor
REM   build.bat run debug    - debug build + run the editor
REM   build.bat check        - cargo check (fast, no codegen)
REM   build.bat clean        - cargo clean
REM   build.bat test         - cargo test (workspace)

setlocal

set "MODE=%~1"
set "EXTRA=%~2"

if /I "%MODE%"=="" goto :build_release
if /I "%MODE%"=="release" goto :build_release
if /I "%MODE%"=="debug" goto :build_debug
if /I "%MODE%"=="check" goto :do_check
if /I "%MODE%"=="clean" goto :do_clean
if /I "%MODE%"=="test" goto :do_test
if /I "%MODE%"=="run" goto :do_run

echo Unknown argument: %MODE%
echo.
echo Usage: build.bat [release^|debug^|run^|check^|clean^|test] [debug]
exit /b 1

:build_release
echo [fluxion] cargo build --workspace --release
cargo build --workspace --release
exit /b %ERRORLEVEL%

:build_debug
echo [fluxion] cargo build --workspace
cargo build --workspace
exit /b %ERRORLEVEL%

:do_check
echo [fluxion] cargo check --workspace
cargo check --workspace
exit /b %ERRORLEVEL%

:do_clean
echo [fluxion] cargo clean
cargo clean
exit /b %ERRORLEVEL%

:do_test
echo [fluxion] cargo test --workspace
cargo test --workspace
exit /b %ERRORLEVEL%

:do_run
if /I "%EXTRA%"=="debug" (
    echo [fluxion] cargo run -p fluxion-editor
    cargo run -p fluxion-editor
) else (
    echo [fluxion] cargo run -p fluxion-editor --release
    cargo run -p fluxion-editor --release
)
exit /b %ERRORLEVEL%
