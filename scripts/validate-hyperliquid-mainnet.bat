@echo off
REM HyperLiquid Mainnet Validation Script
REM This script validates the dual-channel architecture (WebSocket + RPC)
REM and tests all components of the HyperLiquid integration

echo ========================================
echo HyperLiquid Mainnet Validation
echo ========================================
echo.

REM Check if .env file exists
if not exist .env (
    echo ERROR: .env file not found
    echo Please create .env file with PRIVATE_KEY set
    exit /b 1
)

REM Load environment variables
for /f "tokens=*" %%a in ('type .env ^| findstr /v "^#"') do set %%a

REM Verify private key is set
if "%PRIVATE_KEY%"=="" (
    echo ERROR: PRIVATE_KEY not set in .env file
    exit /b 1
)

echo [1/7] Building project...
cargo build --release
if errorlevel 1 (
    echo ERROR: Build failed
    exit /b 1
)
echo ✓ Build successful
echo.

echo [2/7] Running unit tests...
cargo test --release --lib
if errorlevel 1 (
    echo ERROR: Unit tests failed
    exit /b 1
)
echo ✓ Unit tests passed
echo.

echo [3/7] Validating configuration...
echo Checking config/my-config.yml...

REM Check if config file exists
if not exist config\my-config.yml (
    echo ERROR: config/my-config.yml not found
    exit /b 1
)

REM Validate HyperLiquid configuration
findstr /C:"hyperliquid:" config\my-config.yml >nul
if errorlevel 1 (
    echo ERROR: HyperLiquid configuration not found in config file
    exit /b 1
)

findstr /C:"enabled: true" config\my-config.yml >nul
if errorlevel 1 (
    echo WARNING: HyperLiquid may not be enabled
)

findstr /C:"ws_url:" config\my-config.yml >nul
if errorlevel 1 (
    echo ERROR: WebSocket URL not configured
    exit /b 1
)

findstr /C:"rpc_url:" config\my-config.yml >nul
if errorlevel 1 (
    echo ERROR: RPC URL not configured
    exit /b 1
)

echo ✓ Configuration validated
echo.

echo [4/7] Testing WebSocket connection...
echo This will attempt to connect to HyperLiquid WebSocket API
echo Press Ctrl+C after 10 seconds if connection is successful
timeout /t 3 /nobreak >nul

REM Create a temporary test program
echo Testing WebSocket connectivity...
cargo run --release --bin mev-bot -- --config config/my-config.yml >logs\validation-ws-test.log 2>&1 &
set BOT_PID=%ERRORLEVEL%

REM Wait for 10 seconds to check logs
timeout /t 10 /nobreak >nul

REM Check logs for WebSocket connection
findstr /C:"HyperLiquid integration enabled" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: Could not verify HyperLiquid initialization
) else (
    echo ✓ HyperLiquid service initialized
)

findstr /C:"WebSocket connected" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: WebSocket connection not confirmed in logs
) else (
    echo ✓ WebSocket connection established
)

findstr /C:"Subscribed to" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: Trading pair subscriptions not confirmed
) else (
    echo ✓ Trading pair subscriptions active
)

REM Kill the bot process
taskkill /F /IM mev-bot.exe >nul 2>&1
echo.

echo [5/7] Testing RPC connection...
echo Checking RPC endpoint connectivity...

findstr /C:"RPC service started" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: RPC service start not confirmed
) else (
    echo ✓ RPC service started
)

findstr /C:"poll_blockchain_state" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: RPC polling not confirmed
) else (
    echo ✓ RPC blockchain polling active
)

findstr /C:"RPC error" logs\validation-ws-test.log >nul
if not errorlevel 1 (
    echo WARNING: RPC errors detected in logs
    echo Check logs\validation-ws-test.log for details
)
echo.

echo [6/7] Checking metrics...
echo Verifying Prometheus metrics are being collected...

findstr /C:"hyperliquid_ws_connected" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: WebSocket metrics not found
) else (
    echo ✓ WebSocket metrics active
)

findstr /C:"hyperliquid_rpc_calls_total" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: RPC metrics not found
) else (
    echo ✓ RPC metrics active
)

findstr /C:"hyperliquid_trades_received_total" logs\validation-ws-test.log >nul
if errorlevel 1 (
    echo WARNING: Trade metrics not found
) else (
    echo ✓ Trade metrics active
)
echo.

echo [7/7] Monitoring for errors and warnings...
echo Checking logs for critical issues...

findstr /C:"ERROR" logs\validation-ws-test.log >nul
if not errorlevel 1 (
    echo WARNING: Errors detected in logs
    echo Recent errors:
    findstr /C:"ERROR" logs\validation-ws-test.log | more
    echo.
)

findstr /C:"WARN" logs\validation-ws-test.log >nul
if not errorlevel 1 (
    echo INFO: Warnings detected in logs
    echo Recent warnings:
    findstr /C:"WARN" logs\validation-ws-test.log | more
    echo.
)

findstr /C:"mempool" logs\validation-ws-test.log >nul
if not errorlevel 1 (
    echo INFO: Mempool references found (should not be used for HyperLiquid)
    findstr /C:"mempool" logs\validation-ws-test.log | more
    echo.
)
echo.

echo ========================================
echo Validation Summary
echo ========================================
echo.
echo ✓ Build: SUCCESS
echo ✓ Tests: SUCCESS
echo ✓ Configuration: VALIDATED
echo.
echo WebSocket Status: Check logs above
echo RPC Status: Check logs above
echo Metrics Status: Check logs above
echo.
echo Full logs available at: logs\validation-ws-test.log
echo.
echo ========================================
echo Next Steps
echo ========================================
echo.
echo 1. Review the validation log for any warnings or errors
echo 2. Monitor Prometheus metrics at http://localhost:9090
echo 3. Start the bot with: cargo run --release --bin mev-bot -- --config config/my-config.yml
echo 4. Monitor logs in real-time: tail -f logs\mev-bot.log
echo 5. Test transaction submission with small amounts
echo.
echo IMPORTANT: Before submitting real transactions:
echo   - Verify WebSocket is receiving market data
echo   - Verify RPC is polling blockchain state
echo   - Verify opportunity detection is working
echo   - Start with small test amounts
echo.

pause
