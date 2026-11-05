#!/usr/bin/env bash
# assemble-assets.sh: per-artifact file sets, symlinks, unexpected artifacts, tarballs whose
# binary must equal the audited one, SHA256SUMS.
set -euo pipefail
# shellcheck source=lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

V=0.1.0
declare -A target=([amd64]=x86_64-unknown-linux-musl [arm64]=aarch64-unknown-linux-musl)
dl="$TMP/dl"
outdir="$TMP/out"

build_dl() {
  rm -rf "$dl" "$outdir" "$TMP/pack"
  mkdir -p "$dl/rel-sbom-web" "$dl/rel-supply-chain" "$dl/rel-images"
  echo web >"$dl/rel-sbom-web/deskhaven-web-v$V.cdx.json"
  echo n1 >"$dl/rel-supply-chain/THIRD-PARTY-NOTICES.txt"
  echo n2 >"$dl/rel-supply-chain/THIRD-PARTY-NOTICES-relay.txt"
  echo rust >"$dl/rel-supply-chain/deskhaven-v$V.cdx.json"
  echo images >"$dl/rel-images/images.txt"
  local arch t
  for arch in amd64 arm64; do
    t="${target[$arch]}"
    mkdir -p "$dl/rel-deskhaven-$arch" "$dl/rel-relay-$arch" "$dl/bin-deskhaven-$arch" "$TMP/pack/deskhaven-v$V-$t"
    echo "binary of $arch" >"$dl/bin-deskhaven-$arch/deskhaven"
    cp "$dl/bin-deskhaven-$arch/deskhaven" "$TMP/pack/deskhaven-v$V-$t/deskhaven"
    echo license >"$TMP/pack/deskhaven-v$V-$t/LICENSE"
    tar -C "$TMP/pack" -czf "$dl/rel-deskhaven-$arch/deskhaven-v$V-$t.tar.gz" "deskhaven-v$V-$t"
    echo audit >"$dl/rel-deskhaven-$arch/audit-deskhaven-v$V-$t.txt"
    echo audit >"$dl/rel-relay-$arch/audit-hbbr-v$V-$t.txt"
  done
}
assemble() { env VERSION="$V" PUBLISH="${PUBLISH:-true}" bash "$RELEASE_DIR/assemble-assets.sh" "$dl" "$outdir"; }

echo "== a clean set"
build_dl
expect_pass "assembles" assemble
assert_eq "11 assets plus SHA256SUMS" "11" "$(find "$outdir" -type f ! -name SHA256SUMS | wc -l | tr -d ' ')"
assert_eq "bin-* are inputs, not assets" "0" "$(find "$outdir" -name deskhaven -type f | wc -l | tr -d ' ')"
(cd "$outdir" && sha256sum -c SHA256SUMS --quiet) && ok "SHA256SUMS verifies" || bad "SHA256SUMS does not verify"
build_dl
rm -rf "$dl/rel-images"
run_capture a env PUBLISH=false VERSION="$V" bash "$RELEASE_DIR/assemble-assets.sh" "$dl" "$outdir"
assert_eq "a dry run needs no image list" "0" "$a_rc"

echo "== artifact file sets"
build_dl
echo extra >"$dl/rel-relay-amd64/surprise.txt"
expect_fail "an extra file in an artifact" "rel-relay-amd64 holds" assemble
build_dl
rm "$dl/rel-supply-chain/THIRD-PARTY-NOTICES-relay.txt"
expect_fail "a missing file in an artifact" "rel-supply-chain holds" assemble
build_dl
rm -rf "$dl/rel-sbom-web"
expect_fail "a missing artifact" "rel-sbom-web was not downloaded" assemble
build_dl
mkdir "$dl/rel-surprise"
expect_fail "an unexpected artifact" "unexpected artifact rel-surprise" assemble
build_dl
rm "$dl/rel-images/images.txt"
echo x >"$dl/rel-images/other.txt"
expect_fail "wrong file name in rel-images" "rel-images holds" assemble
build_dl
rm "$dl/rel-sbom-web/deskhaven-web-v$V.cdx.json"
ln -s /etc/hostname "$dl/rel-sbom-web/deskhaven-web-v$V.cdx.json"
expect_fail "a symlink instead of a file" "not a regular file" assemble

echo "== tarballs"
build_dl
echo tampered >"$TMP/pack/deskhaven-v$V-${target[arm64]}/deskhaven"
tar -C "$TMP/pack" -czf "$dl/rel-deskhaven-arm64/deskhaven-v$V-${target[arm64]}.tar.gz" "deskhaven-v$V-${target[arm64]}"
expect_fail "a tarball whose binary differs from the audited one" "differs from the audited one" assemble
build_dl
ln -sf /bin/sh "$TMP/pack/deskhaven-v$V-${target[amd64]}/deskhaven"
tar -C "$TMP/pack" -czf "$dl/rel-deskhaven-amd64/deskhaven-v$V-${target[amd64]}.tar.gz" "deskhaven-v$V-${target[amd64]}"
expect_fail "a tarball with a symlink in it" "not regular files or directories" assemble
build_dl
mkdir -p "$TMP/esc"
echo x >"$TMP/esc/f"
tar -czf "$dl/rel-deskhaven-amd64/deskhaven-v$V-${target[amd64]}.tar.gz" --transform 's|^|../|' -C "$TMP/esc" f
expect_fail "a tarball with a parent-relative path" "parent-relative" assemble
finish
