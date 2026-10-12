@echo off
rem Build and run the release client. Arguments are passed through.
cd /d "%~dp0"
cargo build --release --bin capitol
if errorlevel 1 (
    echo Build failed!
    exit /b 1
)
target\release\capitol.exe %*
