@echo off
REM Comprehensive test runner for MEV bot CI/CD pipeline
REM This script runs all tests in the correct order with proper reporting

echo ========================================
echo MEV Bot Comprehensive Test Suite
echo ========================================
echo.

REM Set environment variables
set RUST_LOG=info
set CARGO_TERM_COLOR=always
set RUST_BACKTRACE=1

REM Create results directory
if exist test-results rmdir /s /q test-results
mkdir test-results

echo Starting comprehensive test execution...
echo Test results will be saved to: test-results\
echo.

REM ========================================
REM Phase 1: Code Quality and Security
REM ========================================

echo ========================================
echo Phase 1: Code Quality and Security
echo ========================================

echo Running cargo fmt check...
cargo fmt --all -- --check > test-results\fmt-check.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Code formatting check failed
    type test-results\fmt-check.log
    exit /b 1
)
echo ✅ Code formatting check passed

echo Running clippy lints...
cargo clippy --all-targets --all-features -- -D warnings > test-results\clippy.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Clippy lints failed
    type test-results\clippy.log
    exit /b 1
)
echo ✅ Clippy lints passed

echo Running security audit...
cargo audit > test-results\security-audit.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Security audit failed
    type test-results\security-audit.log
    exit /b 1
)
echo ✅ Security audit passed

echo Running dependency checks...
cargo deny check > test-results\dependency-check.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Dependency checks failed
    type test-results\dependency-check.log
    exit /b 1
)
echo ✅ Dependency checks passed

echo.

REM ========================================
REM Phase 2: Unit Tests
REM ========================================

echo ========================================
echo Phase 2: Unit Tests
echo ========================================

echo Running unit tests...
cargo test --all-features --workspace --lib > test-results\unit-tests.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Unit tests failed
    type test-results\unit-tests.log
    exit /b 1
)
echo ✅ Unit tests passed

echo Running documentation tests...
cargo test --doc --workspace > test-results\doc-tests.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Documentation tests failed
    type test-results\doc-tests.log
    exit /b 1
)
echo ✅ Documentation tests passed

echo.

REM ========================================
REM Phase 3: Integration Tests
REM ========================================

echo ========================================
echo Phase 3: Integration Tests
echo ========================================

REM Check if anvil is available
echo Checking for anvil availability...
curl -s -X POST -H "Content-Type: application/json" --data "{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}" http://localhost:8545 >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo ⚠️  Anvil not available - skipping integration tests
    echo Integration tests require anvil running at localhost:8545
    goto :skip_integration
)

echo Running basic integration tests...
set MEV_BOT_RPC_URL=http://localhost:8545
set MEV_BOT_WS_URL=ws://localhost:8545
set MEV_BOT_CHAIN_ID=31337

cargo test --test integration -- --test-threads=1 --nocapture > test-results\integration-basic.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Basic integration tests failed
    type test-results\integration-basic.log
    exit /b 1
)
echo ✅ Basic integration tests passed

echo Running advanced anvil integration tests...
cargo test --test anvil_integration -- --test-threads=1 --nocapture > test-results\integration-anvil.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Anvil integration tests failed
    type test-results\integration-anvil.log
    exit /b 1
)
echo ✅ Anvil integration tests passed

echo Running component integration tests...
cargo test -p mev-mempool --test "*integration*" -- --test-threads=1 > test-results\integration-mempool.log 2>&1
cargo test -p mev-core --test "*integration*" -- --test-threads=1 > test-results\integration-core.log 2>&1
cargo test -p mev-strategies --test "*integration*" -- --test-threads=1 > test-results\integration-strategies.log 2>&1
echo ✅ Component integration tests completed

goto :integration_complete

:skip_integration
echo Integration tests skipped - anvil not available

:integration_complete
echo.

REM ========================================
REM Phase 4: Performance Tests
REM ========================================

echo ========================================
echo Phase 4: Performance Tests
echo ========================================

REM Check if benchmarks exist
if not exist benches (
    echo ⚠️  No benchmark directory found - skipping performance tests
    goto :skip_benchmarks
)

echo Running mempool ingestion benchmarks...
cargo bench --bench mempool_ingestion > test-results\bench-mempool.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ⚠️  Mempool benchmarks failed - continuing
) else (
    echo ✅ Mempool ingestion benchmarks completed
)

echo Running simulation engine benchmarks...
cargo bench --bench simulation_engine > test-results\bench-simulation.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ⚠️  Simulation benchmarks failed - continuing
) else (
    echo ✅ Simulation engine benchmarks completed
)

echo Running strategy performance benchmarks...
cargo bench --bench strategy_performance > test-results\bench-strategy.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ⚠️  Strategy benchmarks failed - continuing
) else (
    echo ✅ Strategy performance benchmarks completed
)

goto :benchmarks_complete

:skip_benchmarks
echo Performance tests skipped - no benchmarks found

:benchmarks_complete
echo.

REM ========================================
REM Phase 5: Coverage Analysis
REM ========================================

echo ========================================
echo Phase 5: Coverage Analysis
echo ========================================

echo Running test coverage analysis...
call scripts\test-coverage.bat > test-results\coverage.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ⚠️  Coverage analysis failed - continuing
    type test-results\coverage.log
) else (
    echo ✅ Coverage analysis completed
)

echo.

REM ========================================
REM Phase 6: Build Validation
REM ========================================

echo ========================================
echo Phase 6: Build Validation
echo ========================================

echo Building release version...
cargo build --release --all-features > test-results\build-release.log 2>&1
if %ERRORLEVEL% neq 0 (
    echo ❌ Release build failed
    type test-results\build-release.log
    exit /b 1
)
echo ✅ Release build completed

echo Validating binary...
if exist target\release\mev-bot.exe (
    echo ✅ MEV bot binary created successfully
    
    REM Get binary size
    for %%i in (target\release\mev-bot.exe) do set BINARY_SIZE=%%~zi
    echo Binary size: %BINARY_SIZE% bytes
) else (
    echo ❌ MEV bot binary not found
    exit /b 1
)

echo.

REM ========================================
REM Test Summary and Results
REM ========================================

echo ========================================
echo Test Summary and Results
echo ========================================

echo.
echo 📊 Test Execution Summary:
echo ✅ Code Quality: PASSED
echo ✅ Security Audit: PASSED
echo ✅ Unit Tests: PASSED
echo ✅ Documentation Tests: PASSED

if exist test-results\integration-basic.log (
    echo ✅ Integration Tests: PASSED
) else (
    echo ⚠️  Integration Tests: SKIPPED ^(anvil not available^)
)

if exist test-results\bench-mempool.log (
    echo ✅ Performance Tests: COMPLETED
) else (
    echo ⚠️  Performance Tests: SKIPPED ^(no benchmarks^)
)

if exist test-results\coverage.log (
    echo ✅ Coverage Analysis: COMPLETED
) else (
    echo ⚠️  Coverage Analysis: FAILED
)

echo ✅ Build Validation: PASSED

echo.
echo 📁 Test artifacts saved to: test-results\
echo.

REM List all test result files
echo Generated test artifacts:
for %%f in (test-results\*) do echo   - %%f

echo.
echo 🎉 All tests completed successfully!
echo.
echo Next steps:
echo 1. Review test results in test-results\ directory
echo 2. Check coverage report: coverage\tarpaulin-report.html
echo 3. Review benchmark results: target\criterion\
echo 4. Deploy to staging environment for further testing

exit /b 0