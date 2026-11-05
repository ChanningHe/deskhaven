# Run the binary with systemd

When you finish, deskhaven and the relay run as hardened systemd services, behind your
nginx.

You need the `deskhaven` binary for your architecture from the
[v0.1.0 release](https://github.com/ChanningHe/deskhaven/releases/tag/v0.1.0), `hbbr` from
the [rustdesk-server 1.1.16 release](https://github.com/rustdesk/rustdesk-server/releases/tag/1.1.16),
nginx with a certificate for your domain, and the [ports](../reference/ports.md) 80, 443
and 21115–21117 open. Run the commands as root.

## 1. Install deskhaven

```sh
base=https://raw.githubusercontent.com/ChanningHe/deskhaven/v0.1.0/deploy
install -m 0755 deskhaven /usr/local/bin/deskhaven
useradd --system --home-dir /var/lib/deskhaven --shell /usr/sbin/nologin deskhaven
curl -fsSL -o /etc/systemd/system/deskhaven.service "$base/systemd/deskhaven.service"
install -d -m 0755 /etc/deskhaven
cat > /etc/deskhaven/deskhaven.toml <<'EOF'
public_url = "https://rd.example.com"

[settings]
"signal.relay_servers" = ["rd.example.com:21117"]
EOF
systemctl daemon-reload && systemctl enable --now deskhaven
```

Replace `rd.example.com` in `deskhaven.toml` with your domain.

## 2. Install the relay

The relay gets deskhaven's public key:

```sh
install -m 0755 hbbr /usr/local/bin/hbbr
key=$(runuser -u deskhaven -- deskhaven key show | head -n1) && [ -n "$key" ] \
  && printf 'KEY=%s\n' "$key" > /etc/deskhaven/relay.env
curl -fsSL -o /etc/systemd/system/hbbr.service "$base/systemd/hbbr.service"
systemctl daemon-reload && systemctl enable --now hbbr
```

## 3. Firewall the local ports

21118 and 21119 listen on all interfaces but are for nginx only, and 21117 has a command
console for local connections. Add the nftables table from the header of
[`deskhaven.service`](../../deploy/systemd/deskhaven.service) to your nftables
configuration and reload it.

## 4. Put nginx in front

Set up nginx as in [Use your own reverse proxy](reverse-proxy.md#nginx). deskhaven
already trusts `127.0.0.1` for `X-Forwarded-For`.

## 5. Create the owner

```sh
journalctl -u deskhaven | grep -o 'https://[^ ]*#token=[^ ]*'
```

Open the link and create the owner account. Then [connect clients](../getting-started/connect-clients.md).
