# ---- Stage 1: checks + release builder ------------------------------------
FROM rust:1.88-bookworm AS builder
WORKDIR /build

COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
COPY migrations ./migrations

# VPS deploys reject source that fails the same core Rust quality gates as CI.
# Database-backed integration tests remain a GitHub CI responsibility because
# deployment must never run a test suite against the production database.
RUN cargo fmt --all -- --check \
 && cargo clippy --locked --all-targets -- -D warnings \
 && cargo test --locked --all-targets \
 && cargo build --release --locked

# ---- Stage 2: runtime ------------------------------------------------------
FROM debian:bookworm-slim
ARG BUILD_SHA=unknown
ARG BUILD_DATE=unknown
LABEL org.opencontainers.image.title="CS Mail API" \
      org.opencontainers.image.revision="$BUILD_SHA" \
      org.opencontainers.image.created="$BUILD_DATE"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wget \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /srv
COPY --from=builder /build/target/release/cs-mail-api /srv/cs-mail-api
COPY migrations ./migrations
RUN mkdir -p /srv/attachments && chown -R 10001:10001 /srv/attachments
USER 10001:10001
ENV RUST_LOG=cs_mail_api=info,tower_http=info
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8080/api/health/ready || exit 1
CMD ["/srv/cs-mail-api"]
