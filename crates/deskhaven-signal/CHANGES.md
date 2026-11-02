# Modification log

deskhaven-signal is a modified version of [rustdesk-server](https://github.com/rustdesk/rustdesk-server)
(hbbs), licensed under the GNU Affero General Public License v3.0 (see `upstream-src/LICENSE`).
This crate is distributed under the same license.

## Upstream attribution (as found in rustdesk-server at 873230e)

- `debian/copyright`: `Copyright: Copyright 2022 open-trade <info@rustdesk.com>`, `License: AGPL-3.0`
  ("either version 3 of the License, or (at your option) any later version").
- `Cargo.toml`: `authors = ["rustdesk <info@rustdesk.com>"]` (no `license` field).
- The upstream source files vendored here carry no per-file copyright or license headers,
  so there are none to preserve; the notices above are the only attribution upstream provides.

## Modification log (AGPL-3.0 section 5(a))

Every modification to the upstream sources is recorded below with its date, the files touched
and the task that made it. `UPSTREAM` records the base commit and per-file checksums;
`scripts/verify-upstream.sh` checks the vendored commit against upstream.

- 2026-11-02, M1-01: vendored verbatim at 873230e35a8805de7fd057e7eb763c46f7a174ef (upstream commit
  dated 2026-10-10), in repository commit f25f49240b5e76eeff0cc83d51ac4af324b7ee1f.
  Files (under `upstream-src/`, not yet compiled): `rendezvous_server.rs`, `peer.rs`, `common.rs`,
  `lib.rs`, `LICENSE`. No modifications.
