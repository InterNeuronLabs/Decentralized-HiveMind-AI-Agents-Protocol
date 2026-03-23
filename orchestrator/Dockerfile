# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1: build
# ---------------------------------------------------------------------------
FROM rust:1.82-slim AS builder

# System deps for ring (OpenSSL) and sqlx (postgres-native)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace manifests first so dependency layers cache independently
COPY Cargo.toml Cargo.lock ./
COPY common/Cargo.toml common/
COPY orchestrator/Cargo.toml orchestrator/
COPY node-client/Cargo.toml node-client/

# Create stub lib/main files so `cargo fetch` can resolve all deps without
# building real source code.
RUN mkdir -p common/src orchestrator/src node-client/src \
    && echo "pub fn dummy() {}" > common/src/lib.rs \
    && echo "fn main() {}" > orchestrator/src/main.rs \
    && echo "fn main() {}" > node-client/src/main.rs

# Pre-fetch and build dependencies only — layer is cached on re-builds
RUN cargo build --release -p orchestrator 2>/dev/null || true

# Now copy real sources and rebuild only changed crates
COPY common/src common/src/
COPY orchestrator/src orchestrator/src/
COPY orchestrator/migrations orchestrator/migrations/

# Force a rebuild of our crates (not deps)
RUN touch orchestrator/src/main.rs common/src/lib.rs \
    && cargo build --release -p orchestrator

# ---------------------------------------------------------------------------
# Stage 2: minimal runtime image
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Non-root user for defence-in-depth
RUN useradd -m -u 1001 orchestrator
USER orchestrator
WORKDIR /app

COPY --from=builder /build/target/release/orchestrator /app/orchestrator

EXPOSE 8080

ENTRYPOINT ["/app/orchestrator"]
