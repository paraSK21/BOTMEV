# MEV Bot - PowerShell Startup Script
# This script starts everything: monitoring stack + MEV bot

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "MEV Bot - Complete Startup" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check if Docker is running
try {
    docker version | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Docker not running"
    }
} catch {
    Write-Host "WARNING: Docker is not running. Starting MEV bot in standalone mode..." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "To enable full monitoring stack:" -ForegroundColor Yellow
    Write-Host "  1. Start Docker Desktop" -ForegroundColor Yellow
    Write-Host "  2. Run this script again" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Starting MEV bot in standalone mode..." -ForegroundColor Yellow
    Write-Host ""
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit 0
}

Write-Host "Starting monitoring stack..." -ForegroundColor Green
Write-Host ""

# Start monitoring services
Write-Host "[1/3] Starting Prometheus..." -ForegroundColor Yellow
docker-compose up -d prometheus
if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: Failed to start Prometheus. Continuing with bot only..." -ForegroundColor Yellow
    Write-Host "Starting MEV bot in standalone mode..." -ForegroundColor Yellow
    Write-Host ""
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit 0
}

Write-Host "[2/3] Starting Grafana..." -ForegroundColor Yellow
docker-compose up -d grafana
if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: Failed to start Grafana. Continuing with bot only..." -ForegroundColor Yellow
    Write-Host "Starting MEV bot in standalone mode..." -ForegroundColor Yellow
    Write-Host ""
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit 0
}

Write-Host "[3/3] Starting Loki..." -ForegroundColor Yellow
docker-compose up -d loki
if ($LASTEXITCODE -ne 0) {
    Write-Host "WARNING: Failed to start Loki. Continuing with bot only..." -ForegroundColor Yellow
    Write-Host "Starting MEV bot in standalone mode..." -ForegroundColor Yellow
    Write-Host ""
    cargo run --release --bin mev-bot -- --config config/my-config.yml
    exit 0
}

Write-Host ""
Write-Host "Waiting for services to be ready..." -ForegroundColor Green
Start-Sleep -Seconds 15

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "Services Started Successfully!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""
Write-Host "📊 Monitoring Dashboards:" -ForegroundColor Cyan
Write-Host "  • Grafana:    http://localhost:3001" -ForegroundColor White
Write-Host "    Login:      admin / admin123" -ForegroundColor Gray
Write-Host "  • Prometheus: http://localhost:9090" -ForegroundColor White
Write-Host "  • Loki:       http://localhost:3100" -ForegroundColor White
Write-Host ""
Write-Host "🤖 MEV Bot Endpoints:" -ForegroundColor Cyan
Write-Host "  • Health:     http://localhost:9091/health" -ForegroundColor White
Write-Host "  • Metrics:    http://localhost:9091/metrics" -ForegroundColor White
Write-Host ""
Write-Host "Starting MEV Bot..." -ForegroundColor Green
Write-Host "Press Ctrl+C to stop everything" -ForegroundColor Yellow
Write-Host ""

# Build the MEV bot first to avoid compilation delays
Write-Host "Building MEV bot..." -ForegroundColor Yellow
cargo build --release --bin mev-bot
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Failed to build MEV bot" -ForegroundColor Red
    Write-Host "Please check your Rust installation and try again" -ForegroundColor Red
    Read-Host "Press Enter to continue"
    exit 1
}

Write-Host ""
Write-Host "Starting MEV bot..." -ForegroundColor Green
Write-Host ""

# Start the MEV bot
cargo run --release --bin mev-bot -- --config config/my-config.yml

Write-Host ""
Write-Host "========================================" -ForegroundColor Yellow
Write-Host "Shutting down services..." -ForegroundColor Yellow
Write-Host "========================================" -ForegroundColor Yellow

# Stop monitoring services
docker-compose down

Write-Host ""
Write-Host "All services stopped." -ForegroundColor Green
Read-Host "Press Enter to continue"
