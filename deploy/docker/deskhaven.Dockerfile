# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
#
# deskhaven: one static musl binary (web console embedded) on distroless/static,
# non-root (65532), meant to run with a read-only root and a volume at
# /var/lib/deskhaven. Build from the repository root:
#
#   docker buildx build -f deploy/docker/deskhaven.Dockerfile -t deskhaven .
#
# Multi-arch (amd64, arm64): each platform compiles natively, so a builder
# needs a native node or QEMU for the other architecture.

# Base images are pinned by digest (tag in the comment); bump both together.

# --- web console: platform-independent output, so built once on the build host.
# node:24.21.0-alpine; keep in step with the Node of the web CI job (.github/workflows/ci.yml).
FROM --platform=$BUILDPLATFORM node:24.21.0-alpine@sha256:ebfe2f90462722a7a4de65e91990e97fe0d401c70e0e762c5b53302f905ec1c1 AS web
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
# rust:1.99.1-alpine3.22; keep in step with rust-toolchain.toml (excluded from
# the build context, so rustup uses the image's toolchain instead of
# installing the dev components).
FROM rust:1.99.1-alpine3.22@sha256:7a54a528456cc1cae43aab7a23e346198a68fdde39134e207110d91273ea3cea AS build
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

# --- runtime: gcr.io/distroless/static-debian13:nonroot
FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329
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
