# Restrict the console to trusted addresses

When you finish, only addresses you trust reach the console and the management API; RustDesk
clients keep working from anywhere.

## Pick an approach

- **A separate listener** ([Keep the console off the internet](separate-console.md)),
  reachable only from where you choose through a firewall, an SSH tunnel or a proxy of its
  own. The client address never serves the console, so no rule depends on path matching.
  Prefer this.
- **Path rules at your reverse proxy**, below, when the console must share the public host
  name. They are only as exact as the proxy's path matching.

## Before any address rule

The rule only holds while nothing reaches the backend ports directly. Check that 21114,
21118 and 21119 are not reachable from outside:

- The compose deployment and the systemd unit bind them to `127.0.0.1` already.
- A binary you start yourself listens on `0.0.0.0:21114`: set
  `listen.client_api = "127.0.0.1:21114"`.
- `docker run -p 21114:21114` publishes it on every interface: use `-p 127.0.0.1:21114:21114`.

TCP and UDP 21115-21117 never pass the proxy and carry no management interface.

## What a client needs

| Path | Used for |
| --- | --- |
| `/api/*` | The client API, including `/api/oidc/callback`, which the user's browser opens during a client OIDC sign-in |
| `/ws/id`, `/ws/relay` | WebSocket rendezvous and relay |
| `/healthz` | Optional: health checks from outside |

Everything else is the console (pages and assets) and the management API,
`/admin/api/v1/*`. Of that API, `auth/*` and `setup` are public, `me` and `me/*` serve any
signed-in user (password change, two-factor enrolment, linked sign-ins, sessions), and the
rest is for administrators.

## Choose a policy

**Console only from trusted addresses.** Public: `/api/`, `/ws/id`, `/ws/relay`,
`/healthz`. Everything else: trusted addresses only. Users outside cannot:

- enrol two-factor authentication, so with it enforced their client sign-in fails until they
  enrol from a trusted network;
- change their password or link OIDC sign-ins;
- sign in to the console through OIDC;
- open the first-run setup link.

**Only management from trusted addresses.** Additionally public: the console pages,
`/admin/api/v1/auth/`, `/admin/api/v1/me` and `/admin/api/v1/me/`. Restricted: every other
`/admin/api/` path. Self-service keeps working everywhere. Administrators outside get a 403
on the admin pages, and anyone can try passwords against the console sign-in (it is rate
limited).

## nginx

Start from the [shipped `deskhaven.conf`](reverse-proxy.md#nginx) and keep its default
`server` blocks, its `map`, and its `proxy_set_header` lines at server level. Replace the
`location` blocks of the 443 server with one of the sets below. Two changes make the rules
exact:

- nginx chooses the `location` on the decoded, normalized path, but a `proxy_pass` without a
  URI part forwards the raw request. The backend reads `..` and `%2f` inside a path segment
  as part of a parameter, so `/admin/api/v1/users/%2e%2e%2fme` matches the `me` location and
  still reaches an admin handler. It stays an admin request that needs an admin session, but
  the address boundary is no longer exact. Every `proxy_pass` below therefore has a URI part
  (nginx then forwards the normalized path it matched), and the `if` refuses encoded dots,
  slashes and backslashes and dot segments up front.
- The `if` also refuses a console path that holds a `/` inside a name, such as the address
  book tag `a/b` or an entry ID with a slash, which the console sends as `%2F`. Rename those.

Replace `203.0.113.0/24` with your trusted range.

### Console only from trusted addresses

```nginx
    if ($request_uri ~* "^[^?]*(%2e|%2f|%5c|/\.\.?(/|$))") { return 400; }

    location /api/      { proxy_pass http://127.0.0.1:21114/api/; }
    location = /healthz { proxy_pass http://127.0.0.1:21114/healthz; }
    location /ws/id     { proxy_pass http://127.0.0.1:21118; proxy_read_timeout 120s; }
    location /ws/relay  { proxy_pass http://127.0.0.1:21119; proxy_read_timeout 120s; }

    location / {
        allow 203.0.113.0/24;
        deny all;
        proxy_pass http://127.0.0.1:21114/;
    }
```

### Only management from trusted addresses

```nginx
    if ($request_uri ~* "^[^?]*(%2e|%2f|%5c|/\.\.?(/|$))") { return 400; }

    location /api/               { proxy_pass http://127.0.0.1:21114/api/; }
    location /admin/api/v1/auth/ { proxy_pass http://127.0.0.1:21114/admin/api/v1/auth/; }
    location = /admin/api/v1/me  { proxy_pass http://127.0.0.1:21114/admin/api/v1/me; }
    location /admin/api/v1/me/   { proxy_pass http://127.0.0.1:21114/admin/api/v1/me/; }
    location /ws/id              { proxy_pass http://127.0.0.1:21118; proxy_read_timeout 120s; }
    location /ws/relay           { proxy_pass http://127.0.0.1:21119; proxy_read_timeout 120s; }

    location /admin/api/ {
        allow 203.0.113.0/24;
        deny all;
        proxy_pass http://127.0.0.1:21114/admin/api/;
    }

    location / { proxy_pass http://127.0.0.1:21114/; }
```

The `proxy_set_header` lines stay at server level, where every location inherits them. A
`proxy_set_header` inside one of these locations would drop all of them there, letting a
client's own `X-Forwarded-For` through. Reload with `nginx -s reload` and test from outside
the range: the restricted paths must answer 403.

## Traefik

Extend the [Traefik file](reverse-proxy.md#traefik) (same entry point, certificate resolver
and services). Every router must pin `Host(...)`, and `priority` must be explicit: Traefik's
default is the rule's length, which would put the wrong router first. Keep `sanitizePath`
at its default (`true`).

### Console only from trusted addresses

Replace the `deskhaven` router and add the middleware:

```yaml
http:
  middlewares:
    trusted-only:
      ipAllowList:
        sourceRange: ["203.0.113.0/24"]
  routers:
    deskhaven-public:
      rule: Host(`rd.example.com`) && (PathPrefix(`/api/`) || Path(`/healthz`))
      priority: 100
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven
    deskhaven:
      rule: Host(`rd.example.com`)
      priority: 1
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      middlewares: [trusted-only]
      service: deskhaven
    deskhaven-ws-id:
      rule: Host(`rd.example.com`) && PathPrefix(`/ws/id`)
      priority: 100
      # entryPoints, tls and service as before
    deskhaven-ws-relay:
      rule: Host(`rd.example.com`) && PathPrefix(`/ws/relay`)
      priority: 100
      # entryPoints, tls and service as before
```

### Only management from trusted addresses

Same middleware. Widen the public router, restrict `/admin/api/`, and keep a public router
for the console pages with the lowest priority:

```yaml
  routers:
    deskhaven-public:
      rule: >-
        Host(`rd.example.com`) && (PathPrefix(`/api/`) || Path(`/healthz`)
        || Path(`/admin/api/v1/me`) || PathPrefix(`/admin/api/v1/me/`)
        || PathPrefix(`/admin/api/v1/auth/`))
      priority: 100
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven
    deskhaven-admin:
      rule: Host(`rd.example.com`) && PathPrefix(`/admin/api/`)
      priority: 50
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      middlewares: [trusted-only]
      service: deskhaven
    deskhaven:
      rule: Host(`rd.example.com`)
      priority: 1
      entryPoints: [websecure]
      tls: { certResolver: letsencrypt }
      service: deskhaven
    # deskhaven-ws-id and deskhaven-ws-relay as above, priority 100
```

## The client address the rules see

- **nginx** matches `allow` and `deny` against `$remote_addr`. Behind a CDN or load
  balancer that is the balancer's address; restore the client's with the balancer's own range:

  ```nginx
  set_real_ip_from 198.51.100.0/24;   # the load balancer, never 0.0.0.0/0
  real_ip_header X-Forwarded-For;
  real_ip_recursive on;
  ```

- **Traefik** at the edge: leave `ipStrategy` unset. Behind one more proxy, set
  `ipStrategy.depth: 1` on the middleware and list that proxy under the entry point's
  `forwardedHeaders.trustedIPs`.

## What this does not cover

An address rule stops other networks, not what runs in an administrator's own browser: a
cross-site script or forged request from a trusted address passes it. It also does not
replace [two-factor authentication](hardening.md) for administrators.
