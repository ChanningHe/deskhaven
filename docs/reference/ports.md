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
client has sent anything. Limits per source refuse a new connection (it is closed at once
and the client retries); they only ever affect that source:

| Per source | Value |
| --- | --- |
| New connections to 21116 | 256 at once, then 64 per second |
| Connections not yet past their first message (21116) or handshake (21118) | 64 per source, 256 per IPv6 /48 |
| Connections to 21116 and 21118 | 1024 per source, 4096 per IPv6 /48 |
| Time to send the first message on 21116 | 15 s |

Global caps never refuse a new connection. When one is full, the least valuable existing
connection is closed to let the new one in:

| Global cap | Value | When full, closed first |
| --- | --- | --- |
| Pending connections on 21116 | a quarter of the connection cap | the oldest of the source holding the most |
| Pending connections on 21118 | an eighth of the connection cap | the oldest of the source holding the most |
| Connections to 21116 and 21118 | a quarter of the open-file limit, from 64 up to 32 768 (16 384 with the systemd unit's 65 536) | a pending one, then the connection idle the longest without a registered device, registered devices last |

A source is an IPv4 address or an IPv6 /64. On 21118 the source is the client address your
proxy reports (`security.trusted_proxies`), counted once the WebSocket handshake is done;
before it, connections from a trusted proxy only count towards the 21118 pending cap.
Clients normally send their first message within a round trip, so many devices behind one
NAT address stay well inside these limits. A large office or a carrier-grade NAT that puts
more than 1024 TCP- or WebSocket-mode clients behind one public address will see the extra
ones refused; those clients keep retrying. The limits are fixed in this version. Refusals and
evictions are logged as a warning at most once a minute, with their count.
