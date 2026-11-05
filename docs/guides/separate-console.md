# Keep the console off the internet

When you finish, the console and the management API listen on a port of their own that
only you reach, through an SSH tunnel. Clients keep using the public address.

Create `compose.override.yaml`:

```yaml
services:
  deskhaven:
    environment:
      DESKHAVEN_LISTEN__ADMIN: 0.0.0.0:21113
      DESKHAVEN_LISTEN__ADMIN_PUBLIC_URL: http://localhost:21113
    ports:
      - "127.0.0.1:21113:21113/tcp"
```

Add it to `COMPOSE_FILE` and restart:

```sh
sed -i 's/^COMPOSE_FILE=.*/&:compose.override.yaml/' .env
docker compose up -d --wait
```

The public address no longer serves the console: `https://rd.example.com/` redirects to
`http://localhost:21113`. Open the tunnel from your computer, then that address:

```sh
ssh -L 21113:127.0.0.1:21113 user@rd.example.com
```

On a binary deployment, add to `/etc/deskhaven/deskhaven.toml` and restart deskhaven:

```toml
[listen]
admin = "127.0.0.1:21113"
admin_public_url = "http://localhost:21113"
```
