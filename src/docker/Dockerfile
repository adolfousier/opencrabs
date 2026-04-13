# ── Stage 1: Build ──────────────────────────────────────────────
# Rust nightly + all build deps. Takes ~15min on first build,
# subsequent builds use Docker layer cache.
FROM rust:slim AS builder

# Install build dependencies (matches CI pipeline)
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    clang \
    libclang-dev \
    libasound2-dev \
    libssl-dev \
    cmake \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install nightly toolchain (required for wacore-binary portable_simd)
RUN rustup toolchain install nightly && rustup default nightly

WORKDIR /build

# Cache dependencies: copy manifests first, build a dummy to cache deps
COPY Cargo.toml Cargo.lock* rust-toolchain.toml ./
COPY src/patches/ src/patches/
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && echo 'pub fn lib() {}' > src/lib.rs \
    && cargo build --release 2>/dev/null || true \
    && rm -rf src

# Copy full source and build for real
COPY . .
RUN cargo build --release --all-features \
    && strip target/release/opencrabs

# ── Stage 2: Runtime ───────────────────────────────────────────
# Minimal image — just the binary + runtime deps
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    libgomp1 \
    libasound2 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash opencrabs

# Copy binary from builder
COPY --from=builder /build/target/release/opencrabs /usr/local/bin/opencrabs

# Workspace and config persist via volume
VOLUME /home/opencrabs/.opencrabs

USER opencrabs
WORKDIR /home/opencrabs

ENTRYPOINT ["opencrabs"]
