# MEV Bot Makefile - Windows Compatible
.PHONY: build test run clean docker-up docker-down help

# Default target
all: build

# Build the main MEV bot (release mode)
build:
	cargo build --release --bin mev-bot

# Build for development
build-dev:
	cargo build --bin mev-bot

# Run tests
test:
	cargo test --lib --bin mev-bot

# Run the bot with your config
run:
	cargo run --release --bin mev-bot -- --config config/my-config.yml

# Run with development config
run-dev:
	cargo run --bin mev-bot -- --config config/dev.yml

# Run with testnet config  
run-testnet:
	cargo run --bin mev-bot -- --config config/testnet.yml

# Run with mainnet config
run-mainnet:
	cargo run --release --bin mev-bot -- --config config/mainnet.yml

# Check code
check:
	cargo check --bin mev-bot

# Clean build artifacts
clean:
	cargo clean

# Format code
fmt:
	cargo fmt --all

# Docker commands
docker-up:
	docker-compose up -d

docker-down:
	docker-compose down

docker-logs:
	docker-compose logs -f

# Show help
help:
	cargo --version