# deskhaven

Self-hosted, open-source server for the official RustDesk client: rendezvous (signal) server,
client API (accounts, 2FA, SSO, address books, groups, audit, strategies) and a modern web console,
shipped as a single binary.

> **Status:** early development, not usable yet.

## Development

```sh
nix develop          # toolchain: rust (rust-toolchain.toml), node, pnpm, protoc, libsodium, sqlite, postgres
cargo build
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Commit messages are a single concise line with a conventional prefix and no body,
e.g. `feat(config): add bootstrap loader (M0-03)`.

## License

[AGPL-3.0-only](LICENSE).

deskhaven is an independent project. It is **not affiliated with, endorsed by, or sponsored by**
the RustDesk project or Purslane Ltd. "RustDesk" is used only to describe client compatibility.
