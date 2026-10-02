# syntax=docker/dockerfile:1
# Multi-stage Dockerfile to build and export the tilo release binary

# Stage 1: Build environment
FROM rust:bookworm AS builder

WORKDIR /app

# Install system C libraries required for cairo-rs and pangocairo
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libcairo2-dev \
    libpango1.0-dev \
    libfontconfig1-dev \
    libfreetype6-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency builds by copying manifest files first
COPY Cargo.toml Cargo.lock ./
COPY VERSION ./

# Dummy source to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs \
    && cargo build --release \
    && rm -rf src

# Copy real source code
COPY src ./src

# Build optimized release binary (touch src to invalidate dummy build)
ARG CARGO_FLAGS="--release"
RUN touch src/main.rs src/lib.rs && cargo build ${CARGO_FLAGS}

# Stage 2: Scratch exporter to output the binary to host filesystem via BuildKit
FROM scratch AS export
COPY --from=builder /app/target/release/tilo /tilo

# Stage 3: Minimal runner image for containerized execution (optional)
FROM debian:bookworm-slim AS runtime
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    libcairo2 \
    libpango-1.0-0 \
    libpangocairo-1.0-0 \
    libfontconfig1 \
    libfreetype6 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/tilo /usr/local/bin/tilo
ENTRYPOINT ["/usr/local/bin/tilo"]
