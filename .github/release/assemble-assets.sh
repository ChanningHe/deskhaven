#!/usr/bin/env bash
# Assembles the release assets from the artifacts of the earlier jobs.
#
# Usage: assemble-assets.sh DL_DIR OUT_DIR      (environment: VERSION, PUBLISH=true|false)
#   DL_DIR  one directory per artifact, named as the artifact (downloaded by exact name)
#   OUT_DIR the merged assets plus SHA256SUMS
#
# Each artifact's file set is checked on its own BEFORE merging (a job that smuggled in an
# extra file, or dropped one, fails here under its own name), a name that two artifacts
# both provide is an error, every file must be a regular file (no symlinks), each
# deskhaven tarball is unpacked and its binary compared with the audited binary of the
# same architecture, and SHA256SUMS covers the result.
set -euo pipefail

dl="${1:?usage: assemble-assets.sh DL_DIR OUT_DIR}"
out="${2:?usage: assemble-assets.sh DL_DIR OUT_DIR}"
version="${VERSION:?VERSION is required}"
publish="${PUBLISH:-false}"

fail() {
  echo "::error::$*"
  exit 1
}

declare -A target=([amd64]=x86_64-unknown-linux-musl [arm64]=aarch64-unknown-linux-musl)

# expected ARTIFACT FILE...: the artifact directory holds exactly these files.
declare -A expected=()
expected[rel-sbom-web]="deskhaven-web-v${version}.cdx.json"
expected[rel-supply-chain]="THIRD-PARTY-NOTICES-relay.txt THIRD-PARTY-NOTICES.txt deskhaven-v${version}.cdx.json"
for arch in amd64 arm64; do
  expected["rel-deskhaven-${arch}"]="audit-deskhaven-v${version}-${target[$arch]}.txt deskhaven-v${version}-${target[$arch]}.tar.gz"
  expected["rel-relay-${arch}"]="audit-hbbr-v${version}-${target[$arch]}.txt"
  expected["bin-deskhaven-${arch}"]="deskhaven"
done
if [ "$publish" = true ]; then expected[rel-images]="images.txt"; fi

mkdir -p "$out"
# bin-* are inputs of the checks below, not release assets
merge=()
for artifact in "${!expected[@]}"; do
  dir="${dl}/${artifact}"
  [ -d "$dir" ] || fail "artifact ${artifact} was not downloaded"
  actual="$(cd "$dir" && find . -mindepth 1 -printf '%P\n' | LC_ALL=C sort | paste -sd' ' -)"
  want="$(tr ' ' '\n' <<<"${expected[$artifact]}" | LC_ALL=C sort | paste -sd' ' -)"
  [ "$actual" = "$want" ] || fail "artifact ${artifact} holds '${actual}', expected '${want}'"
  for file in ${expected[$artifact]}; do
    [ -f "${dir}/${file}" ] && [ ! -L "${dir}/${file}" ] || fail "${artifact}/${file} is not a regular file"
  done
  case "$artifact" in bin-*) ;; *) merge+=("$artifact") ;; esac
done
# no extra artifact directories either
for dir in "$dl"/*/; do
  name="$(basename "$dir")"
  [ -n "${expected[$name]:-}" ] || fail "unexpected artifact ${name}"
done

for artifact in "${merge[@]}"; do
  for file in ${expected[$artifact]}; do
    [ ! -e "${out}/${file}" ] || fail "${file} is provided by two artifacts"
    cp "${dl}/${artifact}/${file}" "${out}/${file}"
  done
done

# the tarballs contain the audited binaries
for arch in amd64 arm64; do
  tarball="${out}/deskhaven-v${version}-${target[$arch]}.tar.gz"
  listing="$(tar -tzvf "$tarball")"
  if awk '{ t = substr($1, 1, 1); if (t != "-" && t != "d") bad = 1 } END { exit bad }' <<<"$listing"; then :; else
    fail "${tarball} contains entries that are not regular files or directories"
  fi
  names="$(tar -tzf "$tarball")"
  if grep -Eq '(^/|(^|/)\.\.(/|$))' <<<"$names"; then
    fail "${tarball} contains absolute or parent-relative paths"
  fi
  work="$(mktemp -d)"
  tar -xzf "$tarball" -C "$work" --no-same-owner
  packed="${work}/deskhaven-v${version}-${target[$arch]}/deskhaven"
  audited="${dl}/bin-deskhaven-${arch}/deskhaven"
  [ -f "$packed" ] && [ ! -L "$packed" ] || fail "${tarball} has no regular deskhaven binary"
  cmp "$packed" "$audited" || fail "the binary in ${tarball} differs from the audited one"
  rm -rf "$work"
done

# SHA256SUMS: written outside OUT_DIR first, the listing must not include itself
sums="$(mktemp)"
(cd "$out" && find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort | xargs sha256sum) >"$sums"
chmod 0644 "$sums"
mv "$sums" "${out}/SHA256SUMS"
cat "${out}/SHA256SUMS"
echo "assembled $(find "$out" -type f | wc -l) files in ${out}"
