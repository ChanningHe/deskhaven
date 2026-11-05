# Use your own reverse proxy

When you finish, your existing nginx or Traefik serves deskhaven over HTTPS instead of the
bundled Caddy.

## 1. Run compose without Caddy

Follow the [quick start](../getting-started/quick-start.md), and before step 3 change
`.env`:

```sh
sed -i 's/^COMPOSE_FILE=.*/COMPOSE_FILE=compose.yaml/' .env
echo 'TRUSTED_PROXIES=172.30.114.1/32' >> .env
```

The HTTP ports are then published on `127.0.0.1` only. A proxy on the host reaches them
from the compose network's gateway, `172.30.114.1`, which deskhaven now trusts for
`X-Forwarded-For`. Rootless Docker changes these addresses; see
[Known limitations](../reference/known-limitations.md#client-addresses).

## 2. Route three paths

| Path | Upstream |
| --- | --- |
| `/ws/id` | `http://127.0.0.1:21118` (WebSocket) |
| `/ws/relay` | `http://127.0.0.1:21119` (WebSocket) |
| `/` | `http://127.0.0.1:21114` |

The proxy must set `X-Forwarded-For` to the client address. Without it every client shares
the proxy's address, in sign-in limits and in the audit log.

### nginx

```sh
curl -fsSL -o /etc/nginx/conf.d/deskhaven.conf \
  https://raw.githubusercontent.com/ChanningHe/deskhaven/v0.1.0/deploy/nginx/deskhaven.conf
```

Replace `rd.example.com` and the certificate paths in it, then `nginx -s reload`. Its
first two `server` blocks drop requests for other host names (`default_server`); remove them
if this nginx serves other sites.

### Traefik

A dynamic configuration file for the file provider, with Traefik on the host network.
Use your own entry point and certificate resolver names:

```yaml
http:
  routers:
    deskhaven:
      rule: Host(`rd.example.com`)
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven
    deskhaven-ws-id:
      rule: Host(`rd.example.com`) && PathPrefix(`/ws/id`)
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven-ws-id
    deskhaven-ws-relay:
      rule: Host(`rd.example.com`) && PathPrefix(`/ws/relay`)
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven-ws-relay
  services:
    deskhaven:
      loadBalancer: { servers: [{ url: "http://127.0.0.1:21114" }] }
    deskhaven-ws-id:
      loadBalancer: { servers: [{ url: "http://127.0.0.1:21118" }] }
    deskhaven-ws-relay:
      loadBalancer: { servers: [{ url: "http://127.0.0.1:21119" }] }
```

Traefik sets the forwarded headers itself and replaces any a client sends.
