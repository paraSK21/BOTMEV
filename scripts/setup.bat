@echo off
echo MEV Bot - Day 1 Setup Script
echo =============================

echo.
echo Checking Rust installation...
rustc --version >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Rust not found. Please install Rust first:
    echo https://rustup.rs/
    pause
    exit /b 1
)
echo ✓ Rust found

echo.
echo Checking Docker installation...
docker --version >nul 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Docker not found. Docker Compose won't work.
    echo Install Docker Desktop: https://www.docker.com/products/docker-desktop
) else (
    echo ✓ Docker found
)

echo.
echo Setting up environment...
if not exist ".env" (
    copy ".env.example" ".env"
    echo ✓ Created .env file from template
) else (
    echo ✓ .env file already exists
)

echo.
echo Creating log directory...
if not exist "logs" mkdir logs
echo ✓ Log directory ready

echo.
echo Checking build requirements...
cargo check --workspace >nul 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Build failed. You need Visual Studio Build Tools.
    echo See BUILD_REQUIREMENTS.md for installation instructions.
    echo.
    echo Alternative: Use Docker for development:
    echo   docker-compose up -d
) else (
    echo ✓ Build check passed
)

echo.
echo =============================
echo Setup complete! Next steps:
echo.
echo 1. Configure .env file with your settings
echo 2. Choose your development approach:
echo.
echo   Option A - Native (requires MSVC):
echo     make build
echo     make run
echo.
echo   Option B - Docker (works immediately):
echo     docker-compose up -d
echo.
echo 3. Access services:
echo   - Grafana: http://localhost:3001
echo   - Prometheus: http://localhost:9090
echo.
echo See README.md for detailed instructions.
echo =============================
pause
