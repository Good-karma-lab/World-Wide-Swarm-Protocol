# Multi-stage build for wws-connector
# Stage 1: Build
FROM rust:1.81-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Copy entire source
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY webapp/dist/ webapp/dist/

# Build release binary
RUN cargo build --release --bin wws-connector

# Stage 2: Minimal runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates wget python3 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/wws-connector /usr/local/bin/wws-connector
COPY --from=builder /build/webapp/dist /var/www/wws
COPY docs/SKILL.md /var/www/wws/SKILL.md
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

EXPOSE 9370 9371

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
