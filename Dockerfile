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
# Cache the cargo registry/git and the target dir across builds. Both are cache
# mounts (not image layers), so the binary must be copied OUT to a normal path
# within this same RUN — otherwise it disappears with the mount.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p velos-server \
    && strip target/release/velos-server \
    && cp target/release/velos-server /usr/local/bin/velos-server

# ---- Stage 3: minimal runtime ------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/velos --shell /usr/sbin/nologin velos \
    && install -d -o velos -g velos /data
COPY --from=build /usr/local/bin/velos-server /usr/local/bin/velos-server
USER velos
WORKDIR /data
EXPOSE 8080
# The binary defaults to 127.0.0.1:8080, which is unreachable outside the
# container; bind all interfaces by default.
ENV VELOS_LISTEN=0.0.0.0:8080
# Persist the SQLite datastore in /data. The directory is owned by the non-root
# velos user so a fresh anonymous volume (or a host bind-mount made writable by
# uid/gid of velos) is writable out of the box. Mount a volume here to keep data
# across container restarts, e.g. `-v velos-data:/data`.
ENV VELOS_DB=/data/velos.db
VOLUME ["/data"]
# Probe /healthz via the binary's own --health-check (reads VELOS_LISTEN), so the
# slim image needs no curl/wget. start-period covers first-boot DB init.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["velos-server", "--health-check"]
ENTRYPOINT ["velos-server"]
