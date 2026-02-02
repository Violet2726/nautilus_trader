@echo off
echo ======================================================
echo Nautilus Trader + ThinkTrader Build Script (Windows)
echo ======================================================

:: 1. Check for Rust
rustc --version >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Rust is not installed. 
    echo Please download and install Rustup from https://rustup.rs/
    pause
    exit /b 1
)

:: 2. Ensure Maturin is installed
echo [INFO] Installing build dependencies...
python -m pip install maturin cython numpy packaging

:: 3. Build Nautilus
echo [INFO] Building Nautilus Trader (this may take several minutes)...
:: Using maturin develop to install in editable mode for the venv
maturin develop --release

if %errorlevel% neq 0 (
    echo [ERROR] Build failed. Please check the logs.
    pause
    exit /b 1
)

echo [SUCCESS] Nautilus Trader has been compiled and installed!
echo You can now run the ThinkTrader integration tests.
pause
