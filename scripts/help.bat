@echo off
echo MEV Bot - Available Commands:
echo.
echo Build Commands:
echo   make build          - Build the MEV bot (release mode)
echo   make build-dev      - Build for development
echo.
echo Run Commands:
echo   make run            - Start everything (bot + monitoring dashboards)
echo   make run-bot-only   - Start bot only (no monitoring)
echo   make run-dev        - Run with development config
echo   make run-testnet    - Run with testnet config
echo   make run-mainnet    - Run with mainnet config
echo.
echo Other Commands:
echo   make test           - Run tests
echo   make clean          - Clean build artifacts
echo   make fmt            - Format code
echo   make docker-up      - Start Docker services only
echo   make docker-down    - Stop Docker services
echo.
echo Quick Start:
echo   1. make build
echo   2. make run
echo.
echo Dashboards (after make run):
echo   • Grafana:    http://localhost:3001 (admin/admin123)
echo   • Prometheus: http://localhost:9090
echo   • Health:     http://localhost:9090/health
echo.
pause
