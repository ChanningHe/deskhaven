# syntax=docker/dockerfile:1
#
# deskhaven: one static musl binary (web console embedded) on distroless/static,
# non-root (65532), meant to run with a read-only root and a volume at
# /var/lib/deskhaven. Build from the repository root:
#
#   docker buildx build -f deploy/docker/deskhaven.Dockerfile -t deskhaven .
#
# Multi-arch (amd64, arm64): each platform compiles natively, so a builder
# needs a native node or QEMU for the other architecture.

# Keep in step with rust-toolchain.toml (excluded from the build context, so
# rustup uses the image's toolchain instead of installing the dev components).
ARG RUST_VERSION=1.99.1
# Keep in step with the Node of the web CI job (.github/workflows/ci.yml).
ARG NODE_VERSION=24.21.0

# --- web console: platform-independent output, so built once on the build host.
FROM --platform=$BUILDPLATFORM node:${NODE_VERSION}-alpine AS web
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
# pnpm comes from package.json `packageManager` (version + sha512, verified by
# corepack). Dependency install scripts never run (as in CI and `xtask dist`).
RUN corepack enable pnpm \
 && pnpm install --frozen-lockfile --ignore-scripts
COPY web/ ./
RUN pnpm build

# --- server binary: static musl, native per target platform.
FROM rust:${RUST_VERSION}-alpine3.22 AS build
# make + file: the bundled libsodium build (its configure script needs both).
# OpenSSL: hbb_common's native-tls (tungstenite) links it on Linux; statically here.
RUN apk add --no-cache musl-dev make file pkgconf openssl-dev openssl-libs-static
ENV OPENSSL_STATIC=1
WORKDIR /src
COPY Cargo.toml Cargo.lock LICENSE ./
COPY .cargo/ .cargo/
COPY crates/ crates/
COPY xtask/ xtask/
# Release builds embed web/dist (crates/deskhaven-web).
COPY --from=web /src/web/dist web/dist/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p deskhaven \
 && file target/release/deskhaven | grep -Eq 'static(-pie)? linked' \
 && install -D -m 0755 target/release/deskhaven /out/usr/local/bin/deskhaven \
 && install -D -m 0644 LICENSE /out/usr/share/licenses/deskhaven/LICENSE \
 && install -d -m 0700 -o 65532 -g 65532 /out/var/lib/deskhaven

# --- runtime
FROM gcr.io/distroless/static-debian13:nonroot
ARG VERSION=dev
LABEL org.opencontainers.image.title="deskhaven" \
      org.opencontainers.image.description="Self-hosted server for the RustDesk client" \
      org.opencontainers.image.source="https://github.com/ChanningHe/deskhaven" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build /out/ /
USER 65532:65532
WORKDIR /
VOLUME /var/lib/deskhaven
# 21114 HTTP (client API + console), 21115 NAT test, 21116 rendezvous, 21118 WebSocket.
EXPOSE 21114/tcp 21115/tcp 21116/tcp 21116/udp 21118/tcp
HEALTHCHECK --interval=10s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/deskhaven", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/deskhaven"]
CMD ["serve"]
