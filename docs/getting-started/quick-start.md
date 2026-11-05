# Quick start

When you finish, deskhaven and its relay run on your server behind HTTPS, and you are
signed in to the console as the owner.

## What you need

- A Linux host with Docker Engine 25 or later and Docker Compose 2.24.4 or later.
- A DNS name for the host, for example `rd.example.com`.
- These ports open to the internet: 80/tcp, 443/tcp+udp, 21115/tcp, 21116/tcp+udp and
  21117/tcp ([what each is for](../reference/ports.md)).

## 1. Download the compose files

```sh
mkdir deskhaven && cd deskhaven
base=https://raw.githubusercontent.com/ChanningHe/deskhaven/v0.1.0/deploy/compose
curl -fsSL -O "$base/compose.yaml" -O "$base/compose.caddy.yaml" -O "$base/Caddyfile"
curl -fsSL -o .env "$base/.env.example"
```

## 2. Enter your domain

Set `domain` to your DNS name:

```sh
domain=rd.example.com
sed -i "s/rd\.example\.com/$domain/g" .env
```

## 3. Start deskhaven, then the relay

```sh
docker compose up -d --wait deskhaven
```

The relay only accepts clients that use deskhaven's public key. Hand it the key and start
everything:

```sh
key=$(docker compose exec -T deskhaven deskhaven key show | head -n1) && [ -n "$key" ] \
  && echo "RELAY_KEY=$key" >> .env && docker compose up -d --wait
```

## 4. Create the owner

Print the one-time setup link:

```sh
docker compose logs deskhaven | grep -o 'https://[^ ]*#token=[^ ]*'
```

Open it in your browser and create the owner account. The link works once, for 24 hours;
`docker compose exec deskhaven deskhaven admin create-setup-link` prints a new one.

Next: [connect your clients](connect-clients.md).
