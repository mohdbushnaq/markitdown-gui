@echo off
cd /d "%~dp0"
where cargo >nul 2>nul || (
  echo Rust is not installed. Install it from https://rustup.rs and run this again.
  pause & exit /b 1
)
rem Static C runtime: portable .exe with no extra DLLs
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release || (pause & exit /b 1)
copy /y "target\release\markitdown-gui.exe" "MarkItDown GUI.exe" >nul
echo.
echo Built: "%~dp0MarkItDown GUI.exe"
pause
