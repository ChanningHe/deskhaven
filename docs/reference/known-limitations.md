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

## Signalling

- **Relay requests are not authorised.** Anyone who knows an online device's ID can ask
  the server to relay a connection to it, naming any `host:port` as the relay: the device
  then connects there, which reveals its IP address. The official client sends no licence
  key with this request, so checking one would break every relayed connection. Connection
  authorisation (a session token checked by the server) is planned.
- **A distributed flood can degrade signalling.** The connection limits of 21116 and 21118
  ([Ports](ports.md#connection-limits-on-21116tcp-and-21118tcp)) refuse per source and,
  at their global caps, close the least valuable connection instead of refusing a new one,
  so a flood cannot lock clients out. An attacker with many addresses can still crowd out
  slow handshakes and idle connections, and costs CPU with every accepted connection.
  Flooding at the network level is out of scope: use an upstream firewall or your
  provider's DDoS protection.
- **Registration limits cost memory under a flood.** The per-address limits on key
  registrations remember up to 50 000 source addresses for a day, about 125 MB at worst
  (sources that each registered hundreds of IDs). The compose files set no `mem_limit`.

## WebSocket

- **WebSocket needs wss.** The rendezvous WebSocket (21118) carries no key exchange, so
  only TLS protects it. Clients use `wss://<host>/ws/id` and `/ws/relay` when the ID server
  is entered without a port, as **Client setup** shows it.
- **21119 has no bind setting.** The relay's WebSocket listens on all interfaces. On a
  binary deployment, firewall it (header of `deploy/systemd/deskhaven.service`).
  deskhaven's own 21118 follows `signal.ws_listen`, which that unit sets to
  `127.0.0.1:21118`.

## Client addresses

- **The compose deployment serves the rendezvous ports (21115, 21116) on IPv4 only.** Its
  network has no IPv6, so Docker would pass IPv6 connections through its userland proxy and
  every IPv6 client would arrive from the gateway address. With one address for all of them,
  any client could re-register another IPv6 device's ID to itself and receive the
  connections meant for it, every IPv6 pair would look like one LAN and learn each other's
  local addresses, and whoever learns a device's UUID could replace its key. Dual-stack
  clients use IPv4 instead; IPv6-only clients without NAT64 cannot reach the server.
- **To serve IPv6 clients with their own addresses**, give the network IPv6 (Docker 27 or
  newer, with its default `ip6tables`). Merge into `compose.override.yaml`; without the Caddy
  overlay leave out `caddy` and `environment`. Pick your own ULA prefix if you like:

  ```yaml
  services:
    deskhaven:
      environment:
        DESKHAVEN_SECURITY__TRUSTED_PROXIES: 172.30.114.10/32,fd6d:6b68:7276::10/128
      ports:
        - "[::]:21115:21115/tcp"
        - "[::]:21116:21116/tcp"
        - "[::]:21116:21116/udp"
    caddy:
      networks:
        default:
          ipv4_address: 172.30.114.10
          ipv6_address: fd6d:6b68:7276::10
  networks:
    default:
      enable_ipv6: true
      ipam:
        config:
          - subnet: 172.30.114.0/24
            gateway: 172.30.114.1
            ip_range: 172.30.114.128/25
          - subnet: fd6d:6b68:7276::/64
            ip_range: fd6d:6b68:7276:0:8000::/65
  ```

  Then check under **Devices** that an IPv6 device shows its own last IP: an older Docker, or
  `ip6tables` turned off, still hands every IPv6 client the gateway address, with the
  consequences above.
- **Rootless Docker** gives every client the gateway address, IPv4 included, with the same
  consequences. So do the ports that stay dual-stack on an IPv4-only network for their IPv6
  visitors: Caddy (80, 443) and the relay (21117). There it costs per-address rate limits,
  lockouts and audit addresses, not registrations.
- **Clients behind one public address** (carrier-grade NAT, NAT64) share it, and with it
  the consequences above among themselves. This is how RustDesk's rendezvous protocol works.
- **A reverse proxy on the same host is trusted by address.** Any local process that
  connects from that address can set `X-Forwarded-For` and evade the per-address sign-in
  limits. This applies to a binary deployment (`127.0.0.1/32`) and to `compose.yaml` without
  Caddy behind a host proxy (the network gateway).
- **`/readyz` is public.** It pings the database at most once a second and answers every
  probe in between from that result.

## Accounts and sign-in

- **Recovery codes work in the console only.** The RustDesk client accepts only the
  6-digit code from the authenticator app.
- **A temporary password must be changed in the console only.** The RustDesk client signs
  in with it as is.
- **OIDC sign-ins never ask for the local second factor.** This holds for accounts the
  provider created and for local accounts with a linked sign-in, owners included; **Require
  two-factor authentication** applies to password sign-ins only. Require MFA at the
  identity provider. See [OIDC](../guides/oidc.md#link-an-existing-account).
- **Accounts that sign in only through OIDC cannot link or unlink sign-ins themselves.**
  Linking and unlinking ask for the account's password, which they do not have. An
  administrator unlinks for them; the last linked sign-in of such an account stays.
- **A console sign-in through OIDC may still ask for two-factor enrolment.** A local
  account that **Require two-factor authentication** covers and that has no second factor
  yet signs in, but its console session is limited to setting one up, as after a password
  sign-in.
- **One account holds at most `auth.max_sessions_per_user` sessions** (default 1000). Each
  RustDesk client signed in to the account and each console sign-in is one session; one
  more ends the least recently used, without notice. An account shared by more devices
  than that keeps logging its least used devices out: raise the setting, up to 10000, or
  give the devices their own accounts. The console lists the 200 most recently used.
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
