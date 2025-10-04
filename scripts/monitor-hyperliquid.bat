@echo off
REM HyperLiquid Monitoring Script
REM Monitors the bot's HyperLiquid integration in real-time

echo ========================================
echo HyperLiquid Real-Time Monitor
echo ========================================
echo.

REM Check if bot is running
tasklist /FI "IMAGENAME eq mev-bot.exe" 2>NUL | find /I /N "mev-bot.exe">NUL
if "%ERRORLEVEL%"=="0" (
    echo ✓ Bot is running
) else (
    echo ✗ Bot is not running
    echo Start the bot first with: cargo run --release --bin mev-bot -- --config config/my-config.yml
    pause
    exit /b 1
)

echo.
echo Monitoring Options:
echo 1. View live logs
echo 2. Check WebSocket status
echo 3. Check RPC status
echo 4. View metrics
echo 5. Monitor opportunities
echo 6. Check for errors
echo 7. Full dashboard
echo 8. Exit
echo.

set /p choice="Select option (1-8): "

if "%choice%"=="1" goto live_logs
if "%choice%"=="2" goto ws_status
if "%choice%"=="3" goto rpc_status
if "%choice%"=="4" goto metrics
if "%choice%"=="5" goto opportunities
if "%choice%"=="6" goto errors
if "%choice%"=="7" goto dashboard
if "%choice%"=="8" goto end

:live_logs
echo.
echo ========================================
echo Live Logs (Press Ctrl+C to stop)
echo ========================================
echo.
powershell -Command "Get-Content logs\mev-bot.log -Wait -Tail 50"
goto menu

:ws_status
echo.
echo ========================================
echo WebSocket Status
echo ========================================
echo.
echo Checking WebSocket connection...
findstr /C:"WebSocket connected" logs\mev-bot.log | more
echo.
echo Recent WebSocket events:
findstr /C:"HyperLiquid" logs\mev-bot.log | findstr /C:"trade" | more
echo.
pause
goto menu

:rpc_status
echo.
echo ========================================
echo RPC Status
echo ========================================
echo.
echo Checking RPC connection...
findstr /C:"RPC service started" logs\mev-bot.log | more
echo.
echo Recent RPC calls:
findstr /C:"poll_blockchain_state" logs\mev-bot.log | more
echo.
echo Block updates:
findstr /C:"block_number" logs\mev-bot.log | more
echo.
pause
goto menu

:metrics
echo.
echo ========================================
echo Metrics Summary
echo ========================================
echo.
echo Fetching metrics from Prometheus (http://localhost:9090)...
echo.
curl -s http://localhost:9090/metrics | findstr /C:"hyperliquid"
if errorlevel 1 (
    echo ERROR: Could not fetch metrics
    echo Make sure Prometheus is running on port 9090
) else (
    echo.
    echo ✓ Metrics retrieved successfully
)
echo.
pause
goto menu

:opportunities
echo.
echo ========================================
echo Opportunity Detection
echo ========================================
echo.
echo Recent opportunities detected:
findstr /C:"opportunity" logs\mev-bot.log | more
echo.
echo Recent trades evaluated:
findstr /C:"evaluate" logs\mev-bot.log | more
echo.
pause
goto menu

:errors
echo.
echo ========================================
echo Error Monitor
echo ========================================
echo.
echo Recent errors:
findstr /C:"ERROR" logs\mev-bot.log | more
echo.
echo Recent warnings:
findstr /C:"WARN" logs\mev-bot.log | more
echo.
echo Connection issues:
findstr /C:"disconnect" logs\mev-bot.log | more
findstr /C:"reconnect" logs\mev-bot.log | more
echo.
pause
goto menu

:dashboard
echo.
echo ========================================
echo HyperLiquid Dashboard
echo ========================================
echo.

REM WebSocket Status
echo [WebSocket Status]
findstr /C:"WebSocket connected" logs\mev-bot.log >nul 2>&1
if errorlevel 1 (
    echo   Status: ✗ DISCONNECTED
) else (
    echo   Status: ✓ CONNECTED
)

REM Count trades received
for /f %%i in ('findstr /C:"Received HyperLiquid trade" logs\mev-bot.log ^| find /c /v ""') do set trade_count=%%i
echo   Trades received: %trade_count%

REM Count subscriptions
for /f %%i in ('findstr /C:"Subscribed to" logs\mev-bot.log ^| find /c /v ""') do set sub_count=%%i
echo   Active subscriptions: %sub_count%
echo.

REM RPC Status
echo [RPC Status]
findstr /C:"RPC service started" logs\mev-bot.log >nul 2>&1
if errorlevel 1 (
    echo   Status: ✗ NOT STARTED
) else (
    echo   Status: ✓ RUNNING
)

REM Count RPC calls
for /f %%i in ('findstr /C:"poll_blockchain_state" logs\mev-bot.log ^| find /c /v ""') do set rpc_count=%%i
echo   Polling cycles: %rpc_count%

REM Count block updates
for /f %%i in ('findstr /C:"block_number" logs\mev-bot.log ^| find /c /v ""') do set block_count=%%i
echo   Block updates: %block_count%
echo.

REM Opportunity Detection
echo [Opportunity Detection]
for /f %%i in ('findstr /C:"Found MEV opportunities" logs\mev-bot.log ^| find /c /v ""') do set opp_count=%%i
echo   Opportunities found: %opp_count%

for /f %%i in ('findstr /C:"evaluate_transaction" logs\mev-bot.log ^| find /c /v ""') do set eval_count=%%i
echo   Transactions evaluated: %eval_count%
echo.

REM Error Summary
echo [Error Summary]
for /f %%i in ('findstr /C:"ERROR" logs\mev-bot.log ^| find /c /v ""') do set error_count=%%i
echo   Errors: %error_count%

for /f %%i in ('findstr /C:"WARN" logs\mev-bot.log ^| find /c /v ""') do set warn_count=%%i
echo   Warnings: %warn_count%
echo.

REM Performance
echo [Performance]
echo   Log file size:
dir logs\mev-bot.log | findstr "mev-bot.log"

REM Last update
echo   Last log entry:
powershell -Command "Get-Content logs\mev-bot.log -Tail 1"
echo.

echo ========================================
echo Press any key to refresh dashboard...
pause >nul
goto dashboard

:menu
echo.
echo Press any key to return to menu...
pause >nul
cls
goto start

:end
echo.
echo Exiting monitor...
exit /b 0

:start
cls
echo ========================================
echo HyperLiquid Real-Time Monitor
echo ========================================
echo.
echo Monitoring Options:
echo 1. View live logs
echo 2. Check WebSocket status
echo 3. Check RPC status
echo 4. View metrics
echo 5. Monitor opportunities
echo 6. Check for errors
echo 7. Full dashboard
echo 8. Exit
echo.
set /p choice="Select option (1-8): "
if "%choice%"=="1" goto live_logs
if "%choice%"=="2" goto ws_status
if "%choice%"=="3" goto rpc_status
if "%choice%"=="4" goto metrics
if "%choice%"=="5" goto opportunities
if "%choice%"=="6" goto errors
if "%choice%"=="7" goto dashboard
if "%choice%"=="8" goto end
goto start
