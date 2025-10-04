@echo off
REM MEV Bot - Complete Startup Script
REM This script starts everything: monitoring stack + MEV bot

echo ========================================
echo MEV Bot - Complete Startup
echo ========================================
echo.

REM Check if Docker is running
docker version >nul 2>&1
if %errorlevel% neq 0 (
    echo WARNING: Docker is not running. Starting MEV bot in standalone mode...
    echo.
    echo To enable full monitoring stack:
    echo   1. Start Docker Desktop
    echo   2. Run 'make run' again
    echo.
    echo Starting MEV bot in standalone mode...
    echo.
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit /b 0
)

echo Starting monitoring stack...
echo.

REM Start monitoring services
echo [1/3] Starting Prometheus...
docker-compose up -d prometheus
if %errorlevel% neq 0 (
    echo WARNING: Failed to start Prometheus. Continuing with bot only...
    echo Starting MEV bot in standalone mode...
    echo.
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit /b 0
)

echo [2/3] Starting Grafana...
docker-compose up -d grafana
if %errorlevel% neq 0 (
    echo WARNING: Failed to start Grafana. Continuing with bot only...
    echo Starting MEV bot in standalone mode...
    echo.
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit /b 0
)

echo [3/3] Starting Loki...
docker-compose up -d loki
if %errorlevel% neq 0 (
    echo WARNING: Failed to start Loki. Continuing with bot only...
    echo Starting MEV bot in standalone mode...
    echo.
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit /b 0
)

echo.
echo Waiting for services to be ready...
timeout /t 15 /nobreak >nul

echo.
echo ========================================
echo Services Started Successfully!
echo ========================================
echo.
echo 📊 Monitoring Dashboards:
echo   • Grafana:    http://localhost:3001
echo     Login:      admin / admin123
echo   • Prometheus: http://localhost:9090
echo   • Loki:       http://localhost:3100
echo.
echo 🤖 MEV Bot Endpoints:
echo   • Health:     http://localhost:9090/health
echo   • Metrics:    http://localhost:9090/metrics
echo.
echo Starting MEV Bot...
echo Press Ctrl+C to stop everything
echo.

REM Build the MEV bot first to avoid compilation delays
echo Building MEV bot...
cargo build --release --bin mev-bot
if %errorlevel% neq 0 (
    echo ERROR: Failed to build MEV bot
    echo Please check your Rust installation and try again
    pause
    exit /b 1
)

echo.
echo Starting MEV bot...
echo.

REM Start the MEV bot
cargo run --release --bin mev-bot -- --config config/my-config.yml

echo.
echo ========================================
echo Shutting down services...
echo ========================================

REM Stop monitoring services
docker-compose down

echo.
echo All services stopped.
pause
