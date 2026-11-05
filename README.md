# deskhaven

Self-hosted, open-source server for the official RustDesk client: rendezvous (signal) server,
client API (accounts, 2FA, SSO, address books, groups, audit, strategies) and a modern web console,
shipped as a single binary.

> **Status:** pre-release; the first version, v0.1.0, is not out yet.

User documentation: [docs/](docs/README.md), starting with the [quick start](docs/getting-started/quick-start.md).

## Development

```sh
nix develop          # toolchain: rust (rust-toolchain.toml), node, pnpm, protoc, libsodium, sqlite, postgres
cargo build
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Web console access

The console's session cookies are `__Host-` + `Secure`, so browsers only keep them on HTTPS
pages (or on `http://localhost`). deskhaven itself serves plain HTTP; on a plain-HTTP address
the console refuses to sign in (`insecure_transport`) instead of signing you out right away.
Reach it in one of these ways:

- **HTTPS reverse proxy** (recommended): terminate TLS in front of deskhaven, set `public_url`
  (or `listen.admin_public_url` with a split admin listener) to the https address, and list the
  proxy in `security.trusted_proxies` so its `X-Forwarded-For` / `X-Forwarded-Proto` count.
- **SSH tunnel**: `ssh -L 21114:127.0.0.1:21114 user@server`, then open `http://localhost:21114`.

First run: while no owner exists, the server logs a one-time link (`/setup#token=…`, valid 24 h)
to create the owner account; `deskhaven admin create-setup-link` issues a new one.

## License

[AGPL-3.0-only](LICENSE).

deskhaven is an independent project. It is **not affiliated with, endorsed by, or sponsored by**
the RustDesk project or Purslane Ltd. "RustDesk" is used only to describe client compatibility.
