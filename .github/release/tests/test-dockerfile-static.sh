#!/usr/bin/env bash
# file(1) calls x86_64 musl builds static-pie and aarch64 ones "statically linked".
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

amd64='ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, stripped'
arm64='ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, stripped'
dynamic='ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV), dynamically linked, interpreter /lib/ld-musl-aarch64.so.1, stripped'

checks=0
for dockerfile in "$ROOT"/deploy/docker/*.Dockerfile; do
  name="$(basename "$dockerfile")"
  while IFS= read -r re; do
    checks=$((checks + 1))
    if grep -Eq "$re" <<<"$amd64"; then ok "$name: static-pie (amd64) passes"; else bad "$name: '$re' rejects static-pie"; fi
    if grep -Eq "$re" <<<"$arm64"; then ok "$name: statically linked (arm64) passes"; else bad "$name: '$re' rejects statically linked"; fi
    if grep -Eq "$re" <<<"$dynamic"; then bad "$name: '$re' accepts a dynamic binary"; else ok "$name: dynamically linked fails"; fi
  done < <(sed -nE "s/.*\\| grep -Eq '([^']*linked[^']*)'.*/\\1/p" "$dockerfile")
done
assert_eq "every static check was found (deskhaven 1, relay 2)" "3" "$checks"

finish
