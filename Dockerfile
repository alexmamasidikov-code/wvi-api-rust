# Stage 1: Build
# Pin builder to bookworm so the resulting binary links against the same
# glibc version as the runtime stage (debian:bookworm-slim). rust:latest
# moved to trixie which ships glibc 2.39 and breaks on bookworm.
FROM rust:bookworm AS builder

WORKDIR /app
# Native deps for rdkafka-sys (cmake, sasl, zstd, zlib) + openssl.
RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev cmake build-essential \
    libsasl2-dev zlib1g-dev libzstd-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy full source and build in one stage.
# Two-stage dep-caching was dropped because utoipa-swagger-ui's build.rs
# fetches assets at compile time and fails non-deterministically in Docker.
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release --bin wvi-api

# Stage 2: Runtime
FROM debian:bookworm-slim

# ca-certs for HTTPS, libssl3 for openssl, libsasl2-2 + libzstd1 for rdkafka,
# curl for the HEALTHCHECK, tini for proper PID 1 signal handling.
RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 libsasl2-2 libzstd1 \
    curl tini \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/wvi-api .
COPY migrations/ migrations/

EXPOSE 8091

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://localhost:8091/api/v1/health/server-status || exit 1

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["./wvi-api"]
