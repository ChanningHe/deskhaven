#!/usr/bin/env bash
# Prints `export` lines that point the remote E2E stack at pinned tools from nixpkgs, and the
# store path + sha256 of each client launcher it will run. Use: eval "$(web/e2e/remote/nix-env.sh)"
# Playwright's browsers are not here: their version must match @playwright/test (e2e/README.md).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
lock_rev=$(sed -n '/"nixpkgs": {/,/}/{s/.*"rev": "\([0-9a-f]*\)".*/\1/p}' "$root/flake.lock" | head -n1)
rev_14=${RD_E2E_NIXPKGS_14:-b6018f87da91d19d0ab4cf979885689b469cdd41}
pkgs="github:NixOS/nixpkgs/$lock_rev"

out() { nix build --no-link --print-out-paths "$1" | head -n1; }

c15=$(out "$pkgs#rustdesk-flutter")
c14=$(out "github:NixOS/nixpkgs/$rev_14#rustdesk-flutter")
hbbr=$(out "$pkgs#rustdesk-server")
xvfb=$(out "$pkgs#xvfb")
xterm=$(out "$pkgs#xterm")
xclock=$(out "$pkgs#xclock")
xdotool=$(out "$pkgs#xdotool")
xrandr=$(out "$pkgs#xrandr")
xclip=$(out "$pkgs#xclip")

{
  echo "# rustdesk 1.5.x $c15 sha256 $(sha256sum "$c15/bin/rustdesk" | cut -d' ' -f1)"
  echo "# rustdesk 1.4.x $c14 sha256 $(sha256sum "$c14/bin/rustdesk" | cut -d' ' -f1)"
} >&2

echo "export RD_E2E_CLIENT_15=$c15/bin/rustdesk"
echo "export RD_E2E_CLIENT_14=$c14/bin/rustdesk"
echo "export RD_E2E_HBBR=$hbbr/bin/hbbr"
echo "export RD_E2E_XVFB=$xvfb/bin/Xvfb"
echo "export RD_E2E_XTERM=$xterm/bin/xterm"
echo "export RD_E2E_XCLOCK=$xclock/bin/xclock"
echo "export RD_E2E_XDOTOOL=$xdotool/bin/xdotool"
echo "export RD_E2E_XRANDR=$xrandr/bin/xrandr"
echo "export RD_E2E_XCLIP=$xclip/bin/xclip"
echo "export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true"
