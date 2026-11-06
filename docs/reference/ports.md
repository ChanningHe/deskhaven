# Ports

Every port deskhaven and the relay use, and how the compose deployment exposes it.

| Port | Service | Purpose | Compose deployment |
| --- | --- | --- | --- |
| 80/tcp | Caddy | HTTP: certificate challenge, redirect to HTTPS | public |
| 443/tcp, 443/udp | Caddy | HTTPS (console, client API, wss) | public |
| 21114/tcp | deskhaven | Console and client API (HTTP) | behind Caddy (`/`) |
| 21115/tcp | deskhaven | NAT type test | public, IPv4 only |
| 21116/tcp, 21116/udp | deskhaven | Rendezvous: registration, hole punching | public, IPv4 only |
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

## Connection limits on 21116/tcp and 21118/tcp

Every connection to the rendezvous TCP port costs the server a key exchange offer before the
client has sent anything, so connections are admitted only within these limits; a refused
connection is closed at once and the client retries:

| Limit | Value |
| --- | --- |
| New connections to 21116 per source | 256 at once, then 64 per second |
| Connections to 21116 of one source that have not sent their first message yet | 64 (and 256 per IPv6 /48) |
| Such connections in all | 2048, or 1/16 of the open-file limit if that is more; at the cap the oldest is closed to let the new one in |
| Time to send the first message on 21116 | 15 s |
| Connections to 21116 and 21118 of one source | 1024 |
| Connections to 21116 and 21118 in all | 1/4 of the open-file limit (16 384 with the systemd unit's 65 536), at least 1024 |

A source is an IPv4 address or an IPv6 /64. On 21118 the source is the client address your
proxy reports (`security.trusted_proxies`), counted once the WebSocket handshake is done.
Clients normally send their first message within a round trip, so many devices behind one
NAT address stay well inside these limits. A large office or a carrier-grade NAT that puts
more than 1024 TCP- or WebSocket-mode clients behind one public address will see the extra
ones refused; those clients keep retrying. The limits are fixed in this version. Refusals and
evictions are logged as a warning at most once a minute, with their count.
