# Multi-stage build for MEV Bot
FROM rust:1.75 as builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build the application
RUN cargo build --release --bin mev-bot

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN useradd -m -u 1001 mevbot

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/mev-bot /usr/local/bin/mev-bot

# Create necessary directories
RUN mkdir -p /app/config /app/logs && \
    chown -R mevbot:mevbot /app

# Switch to app user
USER mevbot

# Expose port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pgrep mev-bot || exit 1

# Run the application
CMD ["mev-bot", "--profile", "docker"]
