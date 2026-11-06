# Back up and upgrade

When you finish, you have a backup you can restore, and deskhaven runs a newer version.

## What to back up

The `deskhaven-data` volume holds everything: the key pair, the master key and the SQLite
database. The master key encrypts the secrets in the database (OIDC and LDAP passwords,
two-factor seeds, address book passwords, proxy passwords in strategies); a database
without it loses them.

Whoever holds both the master key and the database reads every one of those secrets, so a
backup of the whole volume carries the key next to what it protects. Keep a copy of the key
apart from your database backups, for example in a password manager, and
[move it out of the volume](#keep-the-master-key-apart) so routine backups no longer
contain it.

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

If the master key lives in a secret file (next section), put that file back too before
starting.

## Keep the master key apart

deskhaven reads the master key from `security.master_key` when it is set, and from
`master.key` in the volume only otherwise. A variable ending in `_FILE` takes its value from
a file, and `master.key` is already in that format, so the key can live in a secret file
outside the volume. Run in the `deskhaven` directory:

```sh
docker run --rm -v deskhaven_deskhaven-data:/data:ro busybox cat /data/master.key > master.key
sudo chown 65532:65532 master.key && sudo chmod 0400 master.key
```

Merge this into `compose.override.yaml`
([how](../reference/configuration.md#in-the-compose-deployment)):

```yaml
services:
  deskhaven:
    environment:
      DESKHAVEN_SECURITY__MASTER_KEY_FILE: /run/secrets/master_key
    secrets: [master_key]

secrets:
  master_key:
    file: ./master.key
```

Apply it with `docker compose up -d --wait`, then check that deskhaven uses the file:
`docker compose run --rm deskhaven config check` says to back up "the security.master_key
value" and prints no warning. Only then remove the copy in the volume:

```sh
docker run --rm -v deskhaven_deskhaven-data:/data busybox rm /data/master.key
```

Back up `master.key` from the `deskhaven` directory to a different place than the volume
backups. Without it they cannot be restored.

On a binary deployment, pass the key as a systemd credential: copy
`/var/lib/deskhaven/master.key` to `/etc/deskhaven/master.key` (mode `0400`, owned by root),
add this to the service (`systemctl edit deskhaven`) and restart it:

```ini
[Service]
LoadCredential=master_key:/etc/deskhaven/master.key
Environment=DESKHAVEN_SECURITY__MASTER_KEY_FILE=%d/master_key
```

Once `journalctl -u deskhaven` shows it running without a master key warning, delete
`/var/lib/deskhaven/master.key`.

If the override, credential or secret file is ever dropped, deskhaven finds no key and,
once its database holds accounts, refuses to start ("no master key") instead of making a
new one: restore the key and start again. Only if the key is lost for good, create a new
one yourself and pass it the same way (`openssl rand -base64 32 > master.key`); every
secret sealed under the old key (two-factor seeds, OIDC, LDAP and SMTP passwords, address
book and strategy passwords) is then unreadable and must be entered again.

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
