# Ports

Every port deskhaven and the relay use, and how the compose deployment exposes it.

| Port | Service | Purpose | Compose deployment |
| --- | --- | --- | --- |
| 80/tcp | Caddy | HTTP: certificate challenge, redirect to HTTPS | public |
| 443/tcp, 443/udp | Caddy | HTTPS (console, client API, wss) | public |
| 21114/tcp | deskhaven | Console and client API (HTTP) | behind Caddy (`/`) |
| 21115/tcp | deskhaven | NAT type test | public |
| 21116/tcp, 21116/udp | deskhaven | Rendezvous: registration, hole punching | public |
| 21117/tcp | relay (hbbr) | Relay | public |
| 21118/tcp | deskhaven | Rendezvous over WebSocket | behind Caddy (`/ws/id`) |
| 21119/tcp | relay (hbbr) | Relay over WebSocket | behind Caddy (`/ws/relay`) |

Without the Caddy overlay (`compose.yaml` alone), 21114, 21118 and 21119 are published on
`127.0.0.1` only, for a reverse proxy on the same host.

The rendezvous ports follow `signal.port` (default 21116): the NAT test is one below it, the
WebSocket two above. Clients expect the client API two below it.

Clients must reach 21115–21117 directly, not through a proxy: hole punching needs their
real source address. See [Known limitations](known-limitations.md#client-addresses) for the
setups that lose it.
