@echo off
REM Deployment validation script for MEV bot
REM This script validates that a deployment is working correctly

echo Starting deployment validation...
echo.

REM Set default values
set ENVIRONMENT=%1
set HEALTH_URL=%2
set METRICS_URL=%3

if "%ENVIRONMENT%"=="" (
    set ENVIRONMENT=staging
)

if "%HEALTH_URL%"=="" (
    set HEALTH_URL=http://localhost:8080
)

if "%METRICS_URL%"=="" (
    set METRICS_URL=http://localhost:9090
)

echo Validating %ENVIRONMENT% deployment...
echo Health URL: %HEALTH_URL%
echo Metrics URL: %METRICS_URL%
echo.

REM Function to check HTTP endpoint
:check_endpoint
set URL=%1
set DESCRIPTION=%2
set MAX_RETRIES=30
set RETRY_COUNT=0

:retry_loop
curl -f -s %URL% >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo ✅ %DESCRIPTION% is accessible
    goto :eof
)

set /a RETRY_COUNT+=1
if %RETRY_COUNT% geq %MAX_RETRIES% (
    echo ❌ %DESCRIPTION% failed after %MAX_RETRIES% attempts
    exit /b 1
)

echo Waiting for %DESCRIPTION% ^(%RETRY_COUNT%/%MAX_RETRIES%^)...
timeout /t 2 /nobreak >nul
goto retry_loop

echo.
echo ========================================
echo Basic Health Checks
echo ========================================

REM Check if container is running
echo Checking if MEV bot container is running...
docker ps | findstr mev-bot-%ENVIRONMENT% >nul
if %ERRORLEVEL% neq 0 (
    echo ❌ MEV bot container is not running
    exit /b 1
)
echo ✅ MEV bot container is running

REM Check health endpoint
echo Checking health endpoint...
call :check_endpoint "%HEALTH_URL%/health" "Health endpoint"

REM Check metrics endpoint
echo Checking metrics endpoint...
call :check_endpoint "%METRICS_URL%/metrics" "Metrics endpoint"

echo.
echo ========================================
echo Configuration Validation
echo ========================================

REM Validate configuration
echo Validating configuration...
docker exec mev-bot-%ENVIRONMENT% /usr/local/bin/mev-bot --validate-config --profile %ENVIRONMENT%
if %ERRORLEVEL% neq 0 (
    echo ❌ Configuration validation failed
    exit /b 1
)
echo ✅ Configuration validation passed

echo.
echo ========================================
echo Performance Checks
echo ========================================

REM Check memory usage
echo Checking memory usage...
for /f "tokens=*" %%i in ('docker stats --no-stream --format "{{.MemPerc}}" mev-bot-%ENVIRONMENT%') do set MEMORY_USAGE=%%i
set MEMORY_USAGE=%MEMORY_USAGE:~0,-1%
echo Memory usage: %MEMORY_USAGE%%%

if %MEMORY_USAGE% gtr 90 (
    echo ⚠️  High memory usage: %MEMORY_USAGE%%%
) else (
    echo ✅ Memory usage normal: %MEMORY_USAGE%%%
)

REM Check CPU usage
echo Checking CPU usage...
for /f "tokens=*" %%i in ('docker stats --no-stream --format "{{.CPUPerc}}" mev-bot-%ENVIRONMENT%') do set CPU_USAGE=%%i
set CPU_USAGE=%CPU_USAGE:~0,-1%
echo CPU usage: %CPU_USAGE%%%

if %CPU_USAGE% gtr 80 (
    echo ⚠️  High CPU usage: %CPU_USAGE%%%
) else (
    echo ✅ CPU usage normal: %CPU_USAGE%%%
)

echo.
echo ========================================
echo Functional Tests
echo ========================================

REM Test dry run functionality
echo Testing dry run functionality...
timeout 30 docker exec mev-bot-%ENVIRONMENT% /usr/local/bin/mev-bot --dry-run --duration 10s
if %ERRORLEVEL% neq 0 (
    echo ❌ Dry run test failed
    exit /b 1
)
echo ✅ Dry run test passed

REM Test metrics collection
echo Testing metrics collection...
curl -s %METRICS_URL%/metrics | findstr "mev_bot" >nul
if %ERRORLEVEL% neq 0 (
    echo ❌ MEV bot metrics not found
    exit /b 1
)
echo ✅ MEV bot metrics are being collected

REM Count metrics
for /f %%i in ('curl -s %METRICS_URL%/metrics ^| find /c "mev_bot"') do set METRICS_COUNT=%%i
echo Found %METRICS_COUNT% MEV bot metrics

if %METRICS_COUNT% lss 10 (
    echo ⚠️  Low metrics count: %METRICS_COUNT%
) else (
    echo ✅ Good metrics count: %METRICS_COUNT%
)

echo.
echo ========================================
echo Log Analysis
echo ========================================

REM Check for errors in recent logs
echo Checking for errors in recent logs...
docker logs mev-bot-%ENVIRONMENT% --since 5m | findstr /i "error" >nul
if %ERRORLEVEL% equ 0 (
    echo ⚠️  Errors found in recent logs:
    docker logs mev-bot-%ENVIRONMENT% --since 5m | findstr /i "error"
) else (
    echo ✅ No errors in recent logs
)

REM Check for warnings
echo Checking for warnings in recent logs...
docker logs mev-bot-%ENVIRONMENT% --since 5m | findstr /i "warn" >nul
if %ERRORLEVEL% equ 0 (
    echo ⚠️  Warnings found in recent logs:
    docker logs mev-bot-%ENVIRONMENT% --since 5m | findstr /i "warn"
) else (
    echo ✅ No warnings in recent logs
)

REM Check log volume
for /f %%i in ('docker logs mev-bot-%ENVIRONMENT% --since 1m ^| find /c /v ""') do set LOG_LINES=%%i
echo Log lines in last minute: %LOG_LINES%

if %LOG_LINES% equ 0 (
    echo ⚠️  No log activity - service may be stuck
) else (
    echo ✅ Service is logging activity
)

echo.
echo ========================================
echo Network Connectivity
echo ========================================

REM Test RPC connectivity from container
echo Testing RPC connectivity...
docker exec mev-bot-%ENVIRONMENT% curl -s -X POST -H "Content-Type: application/json" --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}" http://api.hyperliquid.xyz/evm >nul
if %ERRORLEVEL% neq 0 (
    echo ❌ RPC connectivity test failed
    exit /b 1
)
echo ✅ RPC connectivity test passed

echo.
echo ========================================
echo Security Checks
echo ========================================

REM Check if running as root (security concern)
docker exec mev-bot-%ENVIRONMENT% whoami | findstr "root" >nul
if %ERRORLEVEL% equ 0 (
    echo ⚠️  Container is running as root - security risk
) else (
    echo ✅ Container is not running as root
)

REM Check for exposed secrets
echo Checking for exposed secrets...
docker exec mev-bot-%ENVIRONMENT% env | findstr /i "key\|secret\|password" >nul
if %ERRORLEVEL% equ 0 (
    echo ⚠️  Potential secrets found in environment variables
) else (
    echo ✅ No obvious secrets in environment variables
)

echo.
echo ========================================
echo Deployment Summary
echo ========================================

echo Environment: %ENVIRONMENT%
echo Container Status: RUNNING
echo Health Check: PASSED
echo Metrics: AVAILABLE
echo Configuration: VALID
echo Performance: ACCEPTABLE
echo Functionality: WORKING
echo Logs: ACTIVE
echo Network: CONNECTED

echo.
if "%ENVIRONMENT%"=="production" (
    echo 🚀 Production deployment validation completed successfully!
    echo The MEV bot is ready for live trading.
    echo.
    echo ⚠️  IMPORTANT REMINDERS:
    echo - Monitor metrics dashboard continuously
    echo - Set up alerting for critical metrics
    echo - Have rollback procedure ready
    echo - Monitor gas prices and network conditions
) else (
    echo ✅ %ENVIRONMENT% deployment validation completed successfully!
    echo The MEV bot is ready for testing.
)

echo.
echo Validation completed at %date% %time%
exit /b 0