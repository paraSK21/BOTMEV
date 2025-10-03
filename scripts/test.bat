@echo off
REM Comprehensive testing script for MEV bot

echo Starting MEV Bot Test Suite
echo =============================

REM Set test environment variables
set MEV_BOT_RPC_URL=http://localhost:8545
set MEV_BOT_WS_URL=ws://localhost:8545
set MEV_BOT_CHAIN_ID=31337
set RUST_LOG=debug
set RUST_BACKTRACE=1

REM Check if anvil is running
echo Checking for Anvil...
curl -s -X POST -H "Content-Type: application/json" --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}" http://localhost:8545 >nul 2>&1
if %errorlevel% neq 0 (
    echo Warning: Anvil not detected at localhost:8545
    echo Some integration tests may be skipped
    echo.
)

REM Run formatting check
echo Running formatting check...
cargo fmt --all -- --check
if %errorlevel% neq 0 (
    echo ERROR: Code formatting issues found
    exit /b 1
)
echo ✅ Formatting check passed
echo.

REM Run clippy
echo Running Clippy lints...
cargo clippy --all-targets --all-features -- -D warnings
if %errorlevel% neq 0 (
    echo ERROR: Clippy lints failed
    exit /b 1
)
echo ✅ Clippy check passed
echo.

REM Run security audit
echo Running security audit...
cargo audit
if %errorlevel% neq 0 (
    echo ERROR: Security audit failed
    exit /b 1
)
echo ✅ Security audit passed
echo.

REM Run unit tests
echo Running unit tests...
cargo test --lib --bins --workspace --verbose
if %errorlevel% neq 0 (
    echo ERROR: Unit tests failed
    exit /b 1
)
echo ✅ Unit tests passed
echo.

REM Run doc tests
echo Running documentation tests...
cargo test --doc --workspace
if %errorlevel% neq 0 (
    echo ERROR: Documentation tests failed
    exit /b 1
)
echo ✅ Documentation tests passed
echo.

REM Run integration tests
echo Running integration tests...
cargo test --test integration -- --test-threads=1 --nocapture
if %errorlevel% neq 0 (
    echo WARNING: Integration tests failed (may be expected without Anvil)
) else (
    echo ✅ Integration tests passed
)
echo.

REM Run benchmarks (quick mode)
echo Running performance benchmarks...
cargo bench --bench performance -- --quick
if %errorlevel% neq 0 (
    echo WARNING: Benchmarks failed
) else (
    echo ✅ Benchmarks completed
)
echo.

REM Generate coverage report if tarpaulin is available
where cargo-tarpaulin >nul 2>&1
if %errorlevel% equ 0 (
    echo Generating coverage report...
    cargo tarpaulin --all-features --workspace --timeout 300 --out Html --output-dir ./coverage
    if %errorlevel% equ 0 (
        echo ✅ Coverage report generated in ./coverage/
    ) else (
        echo WARNING: Coverage report generation failed
    )
) else (
    echo Skipping coverage report (cargo-tarpaulin not installed)
)
echo.

echo =============================
echo Test Suite Completed
echo =============================
echo.
echo To install missing tools:
echo   cargo install cargo-tarpaulin
echo   cargo install cargo-audit
echo   cargo install cargo-deny
echo.
echo To run specific test categories:
echo   cargo test --lib                    (unit tests only)
echo   cargo test --test integration       (integration tests only)
echo   cargo bench --bench performance     (benchmarks only)
echo   cargo tarpaulin --out Html          (coverage report)