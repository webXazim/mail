# ---- Stage 1: builder ------------------------------------------------------
# Rust toolchain inside Docker = no native install needed on dev/VPS machines.
# Toolchain pinned via rust-toolchain.toml for reproducibility.
FROM rust:1.88-bookworm AS builder
WORKDIR /build
COPY rust-toolchain.toml ./
COPY Cargo.toml ./
COPY src ./src
# Migrations are embedded at runtime via sqlx::migrate!(); copy for the macro.
COPY migrations ./migrations
# Pre-fetch deps, then build (cache-friendly: deps layer before src actually in
# real projects; here we accept the smaller cache for a clean skeleton).
RUN cargo build --release

# ---- Stage 2: runtime ------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*
WORKDIR /srv
COPY --from=builder /build/target/release/harbor-api /srv/harbor-api
COPY migrations ./migrations
# Never run the API as root. Numeric user (no shell, no /etc/passwd entry
# required); it only serves TCP and reads its own binary.
USER 10001:10001
ENV RUST_LOG=harbor_api=info,tower_http=info
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:8080/api/health || exit 1
CMD ["/srv/harbor-api"]
