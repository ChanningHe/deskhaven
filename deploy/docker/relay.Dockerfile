# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
#
# deskhaven-relay: the upstream RustDesk relay (hbbr), built unmodified from a
# pinned rustdesk-server release, static musl, on distroless/static, non-root
# (65532), read-only root. Build from the repository root:
#
#   docker buildx build -f deploy/docker/relay.Dockerfile -t deskhaven-relay .
#
# The entrypoint is relay-key-guard (relay-key-guard.rs): it refuses to start
# unless KEY holds deskhaven's PUBLIC key (`deskhaven key show`), then replaces
# itself with hbbr. hbbr alone would fail open: no key, or `-k -` / `-k _` on a
# read-only root, relays for every client.
#
# Base images are pinned by digest (tag in the comment); bump both together.

ARG UPSTREAM_REPO=https://github.com/rustdesk/rustdesk-server
ARG UPSTREAM_TAG=1.1.16
# The commit UPSTREAM_TAG pointed to when pinned; a moved tag fails the build.
ARG UPSTREAM_COMMIT=73523b31cfd25d77dee862e6fc9f5e1fb5e485ef

# rust:1.90.0-alpine3.22: the toolchain upstream's release workflow builds this
# tag with (.github/workflows/build.yaml); its Cargo.lock predates newer compilers.
FROM rust:1.90.0-alpine3.22@sha256:b4b54b176a74db7e5c68fdfe6029be39a02ccbcfe72b6e5a3e18e2c61b57ae26 AS build
ARG UPSTREAM_REPO
ARG UPSTREAM_TAG
ARG UPSTREAM_COMMIT
# git: fetch the source; make + file: the bundled libsodium build (configure).
RUN apk add --no-cache git musl-dev make file
# cargo-auditable embeds the exact dependency list in hbbr (`.dep-v0` section), so
# `cargo audit bin` and Trivy can see its Rust crates; the build output is otherwise the
# unmodified upstream binary.
# renovate: datasource=crate depName=cargo-auditable
ARG CARGO_AUDITABLE_VERSION=0.7.7
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install --locked cargo-auditable --version "${CARGO_AUDITABLE_VERSION}"
WORKDIR /src
RUN git -c advice.detachedHead=false clone --depth 1 --branch "${UPSTREAM_TAG}" \
        --recurse-submodules --shallow-submodules "${UPSTREAM_REPO}" rustdesk-server \
 && test "$(git -C rustdesk-server rev-parse HEAD)" = "${UPSTREAM_COMMIT}"
WORKDIR /src/rustdesk-server
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/rustdesk-server/target \
    cargo auditable build --release --locked --bin hbbr \
 && file target/release/hbbr | grep -Eq 'static(-pie)? linked' \
 && install -D -m 0755 target/release/hbbr /out/usr/local/bin/hbbr \
 && install -D -m 0644 LICENSE /out/usr/share/licenses/rustdesk-server/LICENSE \
 && printf 'hbbr is built unmodified from\n  %s\n  tag %s, commit %s\nLicensed under the GNU AGPL v3 (LICENSE in this directory).\n' \
        "${UPSTREAM_REPO}" "${UPSTREAM_TAG}" "${UPSTREAM_COMMIT}" \
        > /out/usr/share/licenses/rustdesk-server/SOURCE \
 && install -d -m 0755 /out/var/lib/hbbr
# The key guard: std only, so plain rustc; its tests run first.
COPY deploy/docker/relay-key-guard.rs /src/guard/
RUN rustc --edition 2021 --test -o /tmp/guard-tests /src/guard/relay-key-guard.rs \
 && /tmp/guard-tests \
 && rustc --edition 2021 -C opt-level=2 -C strip=symbols \
        -o /out/usr/local/bin/relay-key-guard /src/guard/relay-key-guard.rs \
 && file /out/usr/local/bin/relay-key-guard | grep -Eq 'static(-pie)? linked'

# hbbr and the key guard (`--target binary --output type=local,dest=DIR`): the release
# pipeline audits these files and checks that the pushed image contains exactly them.
FROM scratch AS binary
COPY --from=build /out/usr/local/bin/hbbr /hbbr
COPY --from=build /out/usr/local/bin/relay-key-guard /relay-key-guard

# Third-party notice of hbbr's crates: empty here. The release pipeline replaces this stage
# with `--build-context notices=DIR` (DIR holds THIRD-PARTY-NOTICES.txt, generated from the
# pinned upstream lockfile by .github/release/notices-relay.sh).
FROM scratch AS notices

# gcr.io/distroless/static-debian13:nonroot
FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329
ARG UPSTREAM_REPO
ARG UPSTREAM_TAG
LABEL org.opencontainers.image.title="deskhaven-relay" \
      org.opencontainers.image.description="Upstream RustDesk relay (hbbr), unmodified" \
      org.opencontainers.image.source="${UPSTREAM_REPO}" \
      org.opencontainers.image.version="${UPSTREAM_TAG}" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
COPY --from=build /out/ /
COPY --from=notices / /usr/share/licenses/rustdesk-server/
USER 65532:65532
# hbbr reads `.env`, blacklist.txt and blocklist.txt from its working directory;
# an empty, root-owned directory keeps all three absent.
WORKDIR /var/lib/hbbr
# 21117 relay, 21119 relay WebSocket.
EXPOSE 21117/tcp 21119/tcp
ENTRYPOINT ["/usr/local/bin/relay-key-guard"]
