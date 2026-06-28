# syntax=docker/dockerfile:1
#
# Multi-stage build for the Velos control-plane API server (velos-server).
#
# The dashboard is embedded into the binary at compile time via rust_embed
# (crates/server/src/lib.rs, #[folder = "ui/"]). web/vite.config.ts writes its
# build output to ../crates/server/ui, so the web bundle must be produced before
# the Rust build and placed at crates/server/ui.
#
# Built for a single platform per invocation; CI fans this out across native
# amd64/arm64 runners and merges the results into a multi-arch manifest.

# ---- Stage 1: build the web dashboard ----------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /app/web
# Install deps against the lockfile first for layer caching.
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
# Vite (emptyOutDir) writes the bundle to /app/crates/server/ui.
RUN npm run build

# ---- Stage 2: build velos-server (embeds the UI) -----------------------------
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
# Bring in the freshly built UI so rust_embed bakes it into the release binary.
COPY --from=web /app/crates/server/ui crates/server/ui
RUN cargo build --release --locked -p velos-server
RUN strip target/release/velos-server

# ---- Stage 3: minimal runtime ------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/velos --shell /usr/sbin/nologin velos
COPY --from=build /app/target/release/velos-server /usr/local/bin/velos-server
USER velos
WORKDIR /home/velos
EXPOSE 8080
# The binary defaults to 127.0.0.1:8080, which is unreachable outside the
# container; bind all interfaces by default. VELOS_DB defaults to ./velos.db
# (mount a volume at /home/velos for persistence).
ENV VELOS_LISTEN=0.0.0.0:8080
ENTRYPOINT ["velos-server"]
