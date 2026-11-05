# syntax=docker/dockerfile:1
#
# deskhaven-relay: the upstream RustDesk relay (hbbr), built unmodified from a
# pinned rustdesk-server release, static musl, on distroless/static, non-root
# (65532), read-only root. Only hbbr is shipped. Uses no files from this
# repository, so any context works (the repository root's .dockerignore keeps
# it small):
#
#   docker buildx build -f deploy/docker/relay.Dockerfile -t deskhaven-relay .
#
# Give it only the PUBLIC key (env KEY or `-k <public key>`). Never `-k -` or
# `-k _`: hbbr then reads or creates a private key pair in its working
# directory, and when it cannot write one (read-only root) it runs with no key
# at all, accepting every client.

ARG UPSTREAM_REPO=https://github.com/rustdesk/rustdesk-server
ARG UPSTREAM_TAG=1.1.16
# The commit UPSTREAM_TAG pointed to when pinned; a moved tag fails the build.
ARG UPSTREAM_COMMIT=73523b31cfd25d77dee862e6fc9f5e1fb5e485ef
# The toolchain upstream's release workflow builds this tag with
# (.github/workflows/build.yaml); its Cargo.lock predates newer compilers.
ARG RUST_VERSION=1.90.0

FROM rust:${RUST_VERSION}-alpine3.22 AS build
ARG UPSTREAM_REPO
ARG UPSTREAM_TAG
ARG UPSTREAM_COMMIT
# git: fetch the source; make + file: the bundled libsodium build (configure).
RUN apk add --no-cache git musl-dev make file
WORKDIR /src
RUN git -c advice.detachedHead=false clone --depth 1 --branch "${UPSTREAM_TAG}" \
        --recurse-submodules --shallow-submodules "${UPSTREAM_REPO}" rustdesk-server \
 && test "$(git -C rustdesk-server rev-parse HEAD)" = "${UPSTREAM_COMMIT}"
WORKDIR /src/rustdesk-server
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/rustdesk-server/target \
    cargo build --release --locked --bin hbbr \
 && file target/release/hbbr | grep -Eq 'static(-pie)? linked' \
 && install -D -m 0755 target/release/hbbr /out/usr/local/bin/hbbr \
 && install -D -m 0644 LICENSE /out/usr/share/licenses/rustdesk-server/LICENSE \
 && printf 'hbbr is built unmodified from\n  %s\n  tag %s, commit %s\nLicensed under the GNU AGPL v3 (LICENSE in this directory).\n' \
        "${UPSTREAM_REPO}" "${UPSTREAM_TAG}" "${UPSTREAM_COMMIT}" \
        > /out/usr/share/licenses/rustdesk-server/SOURCE \
 && install -d -m 0755 /out/var/lib/hbbr

FROM gcr.io/distroless/static-debian13:nonroot
ARG UPSTREAM_REPO
ARG UPSTREAM_TAG
LABEL org.opencontainers.image.title="deskhaven-relay" \
      org.opencontainers.image.description="Upstream RustDesk relay (hbbr), unmodified" \
      org.opencontainers.image.source="${UPSTREAM_REPO}" \
      org.opencontainers.image.version="${UPSTREAM_TAG}" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
COPY --from=build /out/ /
USER 65532:65532
# hbbr reads `.env`, blacklist.txt and blocklist.txt from its working directory;
# an empty, root-owned directory keeps all three absent.
WORKDIR /var/lib/hbbr
# 21117 relay, 21119 relay WebSocket.
EXPOSE 21117/tcp 21119/tcp
ENTRYPOINT ["/usr/local/bin/hbbr"]
