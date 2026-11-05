# deskhaven documentation

Start with the three getting-started pages; the guides and the reference are for later.

## Getting started

1. [Quick start](getting-started/quick-start.md): deskhaven running with HTTPS.
2. [Connect clients](getting-started/connect-clients.md): RustDesk clients on your server.
3. [First steps](getting-started/first-steps.md): a user, two-factor authentication, a strategy.

## Guides

- [Use your own reverse proxy](guides/reverse-proxy.md) (nginx, Traefik)
- [Run without a domain](guides/no-domain.md) (LAN, SSH tunnel)
- [Use PostgreSQL](guides/postgresql.md)
- [Keep the console off the internet](guides/separate-console.md)
- [Sign in with OIDC](guides/oidc.md) (Keycloak, Authentik, Google, Entra ID)
- [Sign in with LDAP or Active Directory](guides/ldap.md)
- [Switch from the RustDesk OSS server](guides/migrate-from-oss.md)
- [Run the binary with systemd](guides/binary-systemd.md)
- [Back up and upgrade](guides/backup-upgrade.md)
- [Harden your server](guides/hardening.md)

## Reference

- [Configuration](reference/configuration.md)
- [CLI](reference/cli.md)
- [Ports](reference/ports.md)
- [Known limitations](reference/known-limitations.md)
