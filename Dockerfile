# Global build args
ARG RUST_IMAGE=rust:1.98.1-alpine

# Stage 1: build frontend
# Use --platform=$BUILDPLATFORM to run on the native runner (fast)
FROM --platform=$BUILDPLATFORM node:24-alpine AS frontend

# Wealthfolio Connect configuration (baked into JS bundle at build time)
# Pass via --build-arg to enable; omit to build without Connect.
ARG CONNECT_AUTH_URL=
ARG CONNECT_AUTH_PUBLISHABLE_KEY=
ENV CONNECT_AUTH_URL=${CONNECT_AUTH_URL}
ENV CONNECT_AUTH_PUBLISHABLE_KEY=${CONNECT_AUTH_PUBLISHABLE_KEY}

WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY . .
ENV CI=1
ENV BUILD_TARGET=web
RUN npm install -g pnpm@9.9.0 && pnpm install --frozen-lockfile
# Build only the main app to avoid building workspace addons in this image
RUN pnpm --filter frontend... build && mv dist /web-dist

# Stage 2: build server with cross-compilation
FROM --platform=$BUILDPLATFORM tonistiigi/xx AS xx

# Toolchain shared by the dependency planner and the server build.
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS chef
# Copy xx scripts to handle cross-compilation
COPY --from=xx / /
ARG TARGETPLATFORM

# Install build tools for the HOST (to run cargo, build scripts)
# clang/lld are needed for cross-linking
# pkgconfig is required for openssl-sys to find the target libraries
# `perl` is required by the vendored OpenSSL that SQLCipher links against;
# `build-base` already provides make/gcc.
RUN apk add --no-cache clang lld build-base git file pkgconfig perl

# Install TARGET dependencies
# xx-apk installs into /$(xx-info triple)/...
RUN xx-apk add --no-cache musl-dev gcc openssl-dev openssl-libs-static sqlite-dev

# Install rust target
RUN rustup target add $(xx-cargo --print-target-triple)

# cargo-chef compiles dependencies in their own cached layer, so a source
# change rebuilds only the workspace crates instead of every dependency.
RUN cargo install cargo-chef --locked --version 0.1.78

WORKDIR /app

# Workspace sources as the Rust build sees them
FROM chef AS sources
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/server ./apps/server
# Stub out apps/tauri so the workspace resolves (not built in Docker)
COPY apps/tauri/Cargo.toml apps/tauri/Cargo.toml
RUN mkdir -p apps/tauri/src && echo "fn main(){}" > apps/tauri/src/main.rs && echo "" > apps/tauri/src/lib.rs

# Dependency recipe: changes only when manifests or the lockfile change
FROM sources AS planner
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
# Wealthfolio Connect configuration (baked into server binary at build time)
ARG CONNECT_AUTH_URL=
ARG CONNECT_AUTH_PUBLISHABLE_KEY=
ENV CONNECT_AUTH_URL=${CONNECT_AUTH_URL}
ENV CONNECT_AUTH_PUBLISHABLE_KEY=${CONNECT_AUTH_PUBLISHABLE_KEY}
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV OPENSSL_STATIC=1

# Build dependencies only (cached until the recipe changes)
COPY --from=planner /app/recipe.json recipe.json
RUN xx-cargo chef cook --release --locked -p wealthfolio-server --recipe-path recipe.json

# Now build the server from the full sources
COPY --from=sources /app ./
# Build using xx-cargo which handles target flags
RUN xx-cargo build --locked --release --manifest-path apps/server/Cargo.toml && \
    # Move the binary to a predictable location because the target dir changes with --target
    cp target/$(xx-cargo --print-target-triple)/release/wealthfolio-server /wealthfolio-server

# Final stage
FROM alpine:3.19
WORKDIR /app
# Copy from backend (which is now build platform, but binary is target platform)
COPY --from=backend /wealthfolio-server /usr/local/bin/wealthfolio-server
COPY --from=frontend /web-dist ./dist
ENV WF_DB_PATH=/data/wealthfolio.db
# Wealthfolio Connect API URL (can be overridden at runtime via -e or docker-compose)
ARG CONNECT_API_URL=
ENV CONNECT_API_URL=${CONNECT_API_URL}

# Run as non-root. chown /data BEFORE the VOLUME directive so named volumes
# inherit ownership on first creation. Existing volumes from older images
# need a one-time chown — see docs/self-host/README.md.
RUN addgroup -S -g 1000 wealthfolio \
 && adduser -S -u 1000 -G wealthfolio -H -s /sbin/nologin wealthfolio \
 && mkdir -p /data \
 && chown -R wealthfolio:wealthfolio /data
USER 1000:1000

VOLUME ["/data"]
EXPOSE 8088
CMD ["/usr/local/bin/wealthfolio-server"]
