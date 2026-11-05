# Run without a domain

When you finish, deskhaven runs on a LAN or IP address without HTTPS, and you reach the
console through an SSH tunnel.

## 1. Configure for an IP address

Download the files as in step 1 of the [quick start](../getting-started/quick-start.md),
then set `ip` to the address clients use:

```sh
ip=192.168.1.10
sed -i "s|^DESKHAVEN_PUBLIC_URL=.*|DESKHAVEN_PUBLIC_URL=http://$ip:21114|; s|^DESKHAVEN_RELAY=.*|DESKHAVEN_RELAY=$ip:21117|" .env
```

Remove Caddy from `COMPOSE_FILE` and add `compose.override.yaml` to it
([how](../reference/configuration.md#in-the-compose-deployment)).

Clients sign in through the client API on 21114. Publish it to the network by merging this
into `compose.override.yaml`:

```yaml
services:
  deskhaven:
    ports: !override
      - "21114:21114/tcp"
      - "21115:21115/tcp"
      - "21116:21116/tcp"
      - "21116:21116/udp"
```

Client passwords then cross the network unencrypted.

## 2. Start

Run step 3 of the quick start.

## 3. Open the console through a tunnel

The console signs in over HTTPS or on `localhost` only. From your computer:

```sh
ssh -L 21114:127.0.0.1:21114 user@192.168.1.10
```

Print the setup path on the server and open it on `http://localhost:21114`:

```sh
docker compose logs deskhaven | grep -o '/setup#token=[^ ]*'
```

For example `http://localhost:21114/setup#token=…`. Use the same tunnel for the console
afterwards.

WebSocket connections are not available without HTTPS; see
[Known limitations](../reference/known-limitations.md#websocket).
