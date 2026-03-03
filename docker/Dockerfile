# Stage 1: Build
FROM rust:latest AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY docs/ docs/

# Build release binary
RUN cargo build --release --bin wws-connector

# Stage 2: Runtime
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/wws-connector /usr/local/bin/wws-connector
COPY webapp/dist ./webapp/dist

# Default ports
EXPOSE 9370 9371

ENTRYPOINT ["wws-connector"]
