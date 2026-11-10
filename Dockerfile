# syntax=docker/dockerfile:1

# Keep the container builder on current stable Rust: the locked dependency
# graph includes crates using Cargo's stabilized 2024-edition support.
FROM rust:latest-bookworm AS builder
WORKDIR /app

# Copy manifests first so dependency compilation can be cached by Docker.
COPY Cargo.toml Cargo.lock ./
COPY crates/sandbox-common/Cargo.toml crates/sandbox-common/Cargo.toml
COPY crates/sandbox-core/Cargo.toml crates/sandbox-core/Cargo.toml
COPY crates/sandbox-runtime/Cargo.toml crates/sandbox-runtime/Cargo.toml
COPY crates/sandbox-api/Cargo.toml crates/sandbox-api/Cargo.toml
COPY crates/sandbox-agent/Cargo.toml crates/sandbox-agent/Cargo.toml
COPY crates/sandbox-cli/Cargo.toml crates/sandbox-cli/Cargo.toml
COPY src src
COPY crates crates

RUN cargo build -p sandbox-api --bin cortexenv-server --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget python3-minimal \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 cortexenv

COPY --from=builder /app/target/release/cortexenv-server /usr/local/bin/cortexenv-server

USER cortexenv
ENV CORTEXENV_BIND=0.0.0.0:8080
EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/cortexenv-server"]
