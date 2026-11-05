# Known limitations

Everything deskhaven does not do, or does differently than you might expect, in one place.

## Relay

- **The relay is open.** The upstream relay (hbbr) only checks that a client uses the
  server's public key, and every client has that key. Anyone who knows your server can
  relay traffic through it.
- **The relay has an unauthenticated command console** on 21117 for connections from its
  own loopback address. In the container nothing else shares that address. On a binary
  deployment any local user can reach it; the nftables rules in the header of
  `deploy/systemd/deskhaven.service` limit it to root.
- **The relay's dependencies have known advisories.** hbbr 1.1.16 pins `tungstenite 0.17.2`
  (CVE-2023-43669, a denial of service in the WebSocket handshake), reachable on the relay
  WebSocket (21119).
- **Overriding the relay image's entrypoint** (`--entrypoint`, or `entrypoint:` in compose)
  skips the check that the relay got a valid public key. hbbr without a valid key relays for
  every client.

## WebSocket

- **WebSocket needs wss.** The rendezvous WebSocket (21118) carries no key exchange, so
  only TLS protects it. Clients use `wss://<host>/ws/id` and `/ws/relay` when the ID server
  is entered without a port, as **Client setup** shows it.
- **21118 has no bind setting** and listens on all interfaces. On a binary deployment,
  firewall it (header of `deploy/systemd/deskhaven.service`).

## Client addresses

- **Rootless Docker, and IPv6 clients on an IPv4 Docker network,** reach deskhaven from
  the gateway address instead of their own. Rate limits, lockouts, audit addresses and hole
  punching then see one address for everyone.
- **A reverse proxy on the same host is trusted by address.** Any local process that
  connects from that address can set `X-Forwarded-For` and evade the per-address sign-in
  limits. This applies to a binary deployment (`127.0.0.1/32`) and to `compose.yaml` without
  Caddy behind a host proxy (the network gateway).
- **`/readyz` is public** and queries the database on every request.

## Accounts and sign-in

- **Recovery codes work in the console only.** The RustDesk client accepts only the
  6-digit code from the authenticator app.
- **A temporary password must be changed in the console only.** The RustDesk client signs
  in with it as is.
- **OIDC accounts have no local second factor.** Require MFA at the identity provider.
- **An OIDC identity cannot be linked to an existing account by hand.** It is linked
  automatically only when both sides have a verified email, and never for accounts with
  two-factor authentication, admins or owners.
- **LDAP has no setting for a private CA.** Directory certificates are checked against the
  operating system's CA store; in the container image that holds public CAs only.
- **Sessions outlive directory changes.** An account disabled or a password changed in
  LDAP keeps its deskhaven sessions until they expire or are revoked.
- **Read-only members of a shared address book receive its saved peer passwords.** The
  client needs them in clear to connect.

## Features not there yet

- **Email**: the `smtp.*` settings, `auth.email_code.enabled` and `auth.registration` have
  no effect.
- **Connection ACL**: `signal.acl.default` and `signal.acl.on_error` have no effect.
- **Deployment admission**: `devices.require_deploy` enables `rustdesk --deploy`, but
  devices that were not deployed are not refused.
- **One server per database.** There is no clustering; a second process on the same
  database sees settings changes only after a restart.
- **No move between databases.** Nothing copies data from SQLite to PostgreSQL.
