#!/usr/bin/env bash
# Shared by notices.sh and notices-relay.sh (sourced, not run).

# The crates cargo-about cannot render because they declare no license and ship no license
# file. They are covered by a manual section instead (unlicensed_crates_section). Any OTHER
# crate it cannot resolve fails the build: `cargo about generate --fail` would fail on
# these two as well, so the check is done here.
EXPECTED_UNLICENSED="default_net hbb_common"

section() {
  printf '\n\n%s\n%s\n\n' "$(printf '#%.0s' $(seq 1 80))" "$1"
}

# generate_crate_notice CONFIG TEMPLATE OUTPUT: runs cargo-about in the current directory.
generate_crate_notice() {
  local config="$1" template="$2" output="$3" log skipped
  log="$(mktemp)"
  # cargo-about exits non-zero for a license it cannot satisfy; unresolvable crates are
  # only warnings, collected below.
  cargo about generate --locked --config "$config" --output-file "$output" "$template" 2>"$log" || {
    cat "$log" >&2
    rm -f "$log"
    return 1
  }
  cat "$log" >&2
  skipped="$(sed -nE "s/.*unable to synthesize license expression for '([^' ]+) .*/\1/p" "$log" | sort -u | paste -sd' ' -)"
  rm -f "$log"
  if [ "$skipped" != "$EXPECTED_UNLICENSED" ]; then
    echo "notices: crates without a resolvable license: '${skipped}', expected '${EXPECTED_UNLICENSED}'" >&2
    return 1
  fi
}

unlicensed_crates_section() {
  section "Crates without a license declaration"
  cat <<'EOF'
hbb_common and default_net (both from the RustDesk organization) declare no license and
ship no license file, so they appear in no list above. They are treated as
AGPL-3.0-or-later: for hbb_common this is inferred from rustdesk-server's debian/copyright
("Files: *" under AGPL-3.0, "or any later version"), the tree that carried hbb_common as a
submodule; default_net is assumed to follow the RustDesk client. This is an assumption, not
a grant (see the comments in deny.toml). Sources:
  https://github.com/rustdesk/hbb_common
  https://github.com/rustdesk-org/default_net
EOF
}

native_libraries_section() {
  section "Native libraries linked statically"
  cat <<'EOF'
The crates above wrap, or bundle the sources of, C libraries that end up inside the static
musl binary. Their license texts (Apache-2.0, ISC, MIT) are reproduced in the lists above
under other crates; the copyright holders are:
  OpenSSL 3 (Apache-2.0)      The OpenSSL Project Authors; linked through openssl-sys
  libsodium (ISC)             Frank Denis and contributors; bundled by libsodium-sys
  SQLite (public domain)      D. Richard Hipp and contributors; bundled by libsqlite3-sys
  musl libc (MIT)             Copyright (c) 2005-2020 Rich Felker, et al.
EOF
}

# require_text FILE NEEDLE...: fail loudly if an expected part is missing (template or
# config drift would otherwise ship a notice that silently lacks it).
require_text() {
  local file="$1" needle
  shift
  for needle in "$@"; do
    grep -q "$needle" "$file" || {
      echo "notices: '$needle' missing from $file" >&2
      return 1
    }
  done
}
