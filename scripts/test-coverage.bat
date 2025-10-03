@echo off
REM Comprehensive test coverage script for MEV bot
REM This script runs all tests and generates coverage reports

echo Starting comprehensive test coverage analysis...
echo.

REM Set environment variables
set RUST_LOG=debug
set CARGO_INCREMENTAL=0
set RUSTFLAGS=-Cinstrument-coverage

REM Clean previous artifacts
echo Cleaning previous test artifacts...
cargo clean
if exist coverage rmdir /s /q coverage
mkdir coverage

REM Install required tools
echo Installing coverage tools...
cargo install cargo-tarpaulin --locked
cargo install cargo-llvm-cov --locked

echo.
echo ========================================
echo Running Unit Tests with Coverage
echo ========================================

REM Run unit tests with tarpaulin
echo Running unit tests with tarpaulin...
cargo tarpaulin ^
    --all-features ^
    --workspace ^
    --timeout 300 ^
    --exclude-files "tests/*" ^
    --exclude-files "benches/*" ^
    --exclude-files "*/tests.rs" ^
    --exclude-files "*/test_*.rs" ^
    --exclude-files "*_test.rs" ^
    --out Html ^
    --out Xml ^
    --output-dir ./coverage ^
    --verbose

if %ERRORLEVEL% neq 0 (
    echo Unit tests failed!
    exit /b 1
)

echo.
echo ========================================
echo Running Integration Tests
echo ========================================

REM Check if anvil is available for integration tests
echo Checking for anvil availability...
curl -s -X POST -H "Content-Type: application/json" --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}" http://localhost:8545 >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo Warning: Anvil not available at localhost:8545
    echo Skipping integration tests that require anvil
    goto :skip_integration
)

echo Running integration tests...
set MEV_BOT_RPC_URL=http://localhost:8545
set MEV_BOT_WS_URL=ws://localhost:8545
set MEV_BOT_CHAIN_ID=31337

cargo test --test integration -- --test-threads=1 --nocapture
if %ERRORLEVEL% neq 0 (
    echo Integration tests failed!
    exit /b 1
)

:skip_integration

echo.
echo ========================================
echo Running Benchmark Tests
echo ========================================

REM Check if benchmarks exist
if exist benches (
    echo Running benchmark tests...
    cargo bench --workspace
    if %ERRORLEVEL% neq 0 (
        echo Benchmark tests failed!
        exit /b 1
    )
) else (
    echo No benchmark tests found, skipping...
)

echo.
echo ========================================
echo Running Security and Quality Checks
echo ========================================

REM Run clippy for linting
echo Running clippy lints...
cargo clippy --all-targets --all-features -- -D warnings
if %ERRORLEVEL% neq 0 (
    echo Clippy lints failed!
    exit /b 1
)

REM Run security audit
echo Running security audit...
cargo audit
if %ERRORLEVEL% neq 0 (
    echo Security audit failed!
    exit /b 1
)

REM Run dependency checks
echo Running dependency checks...
cargo deny check
if %ERRORLEVEL% neq 0 (
    echo Dependency checks failed!
    exit /b 1
)

echo.
echo ========================================
echo Running Documentation Tests
echo ========================================

REM Run doc tests
echo Running documentation tests...
cargo test --doc --workspace
if %ERRORLEVEL% neq 0 (
    echo Documentation tests failed!
    exit /b 1
)

echo.
echo ========================================
echo Generating Coverage Reports
echo ========================================

REM Generate additional coverage formats
echo Generating additional coverage formats...

REM Generate lcov format for external tools
cargo tarpaulin ^
    --all-features ^
    --workspace ^
    --timeout 300 ^
    --exclude-files "tests/*" ^
    --exclude-files "benches/*" ^
    --out Lcov ^
    --output-dir ./coverage

REM Generate JSON format for programmatic access
cargo tarpaulin ^
    --all-features ^
    --workspace ^
    --timeout 300 ^
    --exclude-files "tests/*" ^
    --exclude-files "benches/*" ^
    --out Json ^
    --output-dir ./coverage

echo.
echo ========================================
echo Coverage Analysis Complete
echo ========================================

REM Parse coverage results
if exist coverage\cobertura.xml (
    echo Coverage report generated successfully!
    echo HTML report: coverage\tarpaulin-report.html
    echo XML report: coverage\cobertura.xml
    echo LCOV report: coverage\lcov.info
    echo JSON report: coverage\tarpaulin-report.json
    
    REM Extract coverage percentage from XML
    findstr "line-rate" coverage\cobertura.xml > temp_coverage.txt
    if exist temp_coverage.txt (
        echo.
        echo Coverage Summary:
        type temp_coverage.txt
        del temp_coverage.txt
    )
) else (
    echo Warning: Coverage report not found!
)

echo.
echo ========================================
echo Test Summary
echo ========================================

echo ✅ Unit tests: PASSED
echo ✅ Integration tests: PASSED ^(or SKIPPED if anvil unavailable^)
echo ✅ Benchmark tests: PASSED ^(or SKIPPED if no benchmarks^)
echo ✅ Security audit: PASSED
echo ✅ Clippy lints: PASSED
echo ✅ Documentation tests: PASSED
echo ✅ Coverage reports: GENERATED

echo.
echo All tests completed successfully!
echo Coverage reports are available in the 'coverage' directory.
echo Open coverage\tarpaulin-report.html in your browser to view detailed coverage.

exit /b 0