# Back up and upgrade

When you finish, you have a backup you can restore, and deskhaven runs a newer version.

## What to back up

The `deskhaven-data` volume holds everything: the key pair, the master key and the SQLite
database. The master key encrypts the secrets in the database (OIDC and LDAP passwords,
two-factor seeds, address book passwords); a database without it loses them.

With [PostgreSQL](postgresql.md), back up the database and the volume.

## Back up

Run in the `deskhaven` directory. deskhaven stops for a moment so the database is
consistent:

```sh
docker compose stop deskhaven
docker run --rm -v deskhaven_deskhaven-data:/data:ro -v "$PWD":/backup busybox \
  tar czf /backup/deskhaven-$(date +%F).tgz -C /data .
docker compose start deskhaven
```

Keep `.env` and `compose.override.yaml` with it: `.env` holds `RELAY_KEY`, your addresses
and the version.

## Restore

```sh
docker compose down
docker volume rm deskhaven_deskhaven-data
docker compose create deskhaven
docker run --rm -v deskhaven_deskhaven-data:/data -v "$PWD":/backup busybox \
  tar xzpf /backup/deskhaven-2026-01-31.tgz -C /data
docker compose up -d --wait
```

Use the name of your backup file.

On a [binary deployment](binary-systemd.md), the same files are in `/var/lib/deskhaven`:
stop the service and archive that directory.

## Upgrade

`DESKHAVEN_TAG` in `.env` pins the version of both images, and the compose files belong to
the same release. Back up first, then set `version` to the new release and run in the
`deskhaven` directory:

```sh
version=0.2.0
base=https://raw.githubusercontent.com/ChanningHe/deskhaven/v$version/deploy/compose
curl -fsSL -O "$base/compose.yaml" -O "$base/compose.caddy.yaml" -O "$base/Caddyfile"
sed -i "s/^DESKHAVEN_TAG=.*/DESKHAVEN_TAG=$version/" .env
docker compose pull && docker compose up -d --wait
```

Your `.env` and `compose.override.yaml` stay as they are; if the release notes name a new
key in `.env.example`, add it to `.env`. deskhaven upgrades its database at startup.
