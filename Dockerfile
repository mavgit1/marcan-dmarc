# ==============================================================================
# Stage 1: Build SvelteKit static SPA
# ==============================================================================
# Base images pinned by digest (the multi-arch index); Dependabot proposes updates.
FROM node:26-alpine@sha256:6bd9ab43c0ec0190ca7b2d193906cb8b7edf6e7f4afa43ddeccd6452710bffb5 AS web-builder

# pnpm version comes from the packageManager field in web/package.json
RUN corepack enable

WORKDIR /app/web

# Install dependencies with locked versions, under the workspace settings (the
# build scripts allowed, the minimum release age) and .npmrc
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml web/.npmrc ./
RUN pnpm install --frozen-lockfile

# Copy frontend source and compile static SPA to web/build/
COPY web/ ./
RUN pnpm build

# ==============================================================================
# Stage 2: Build Rust backend with embedded SPA
# ==============================================================================
# Keep equal to rust-toolchain.toml
FROM rust:1.98-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS rust-builder

RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static build-base

WORKDIR /app

# Copy Cargo configuration and crate sources
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY migrations/ migrations/

# Copy compiled static assets from web-builder into workspace web/build/
COPY --from=web-builder /app/web/build ./web/build

# Build the release binary
RUN cargo build --release --locked -p dmarc-server

# ==============================================================================
# Stage 3: Minimal production runtime
# ==============================================================================
FROM alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf AS runtime

RUN apk add --no-cache ca-certificates libssl3 sqlite-libs tzdata

WORKDIR /app

# Copy release binary from builder
COPY --from=rust-builder /app/target/release/dmarc-server /usr/local/bin/dmarc-server

# Run as a dedicated unprivileged user; /data (the SQLite database) belongs to it.
# A named volume mounted at /data inherits this ownership; for a bind mount,
# chown the host directory to 10001:10001.
RUN addgroup -S -g 10001 marcan \
    && adduser -S -u 10001 -G marcan -h /data -H marcan \
    && mkdir -p /data \
    && chown marcan:marcan /data
VOLUME /data
USER marcan

EXPOSE 8080

ENV BIND_ADDR=0.0.0.0:8080
ENV DATA_DIR=/data

# Liveness probe (BusyBox wget). Update the URL if you change the port in BIND_ADDR.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD wget -q -O- http://127.0.0.1:8080/healthz || exit 1

CMD ["dmarc-server"]
