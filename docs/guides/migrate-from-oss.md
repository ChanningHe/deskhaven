# Switch from the RustDesk OSS server

When you finish, deskhaven answers your clients with the key pair of your old `hbbs`, so
clients keep their **Key** setting.

You need `id_ed25519` and `id_ed25519.pub` from the old server's data directory, the
working directory of `hbbs`: `/root` in the `rustdesk/rustdesk-server` image (`./data` on
the host with upstream's compose file), `/data` in `rustdesk/rustdesk-server-s6`.

1. Follow the [quick start](../getting-started/quick-start.md) through step 2, and stop the
   old server.
2. In the `deskhaven` directory, with both key files in it, create the container and copy
   the keys into its volume before its first start:

   ```sh
   chmod 600 id_ed25519
   docker compose create deskhaven
   docker compose cp -a id_ed25519 deskhaven:/var/lib/deskhaven/id_ed25519
   docker compose cp -a id_ed25519.pub deskhaven:/var/lib/deskhaven/id_ed25519.pub
   ```

3. Continue with step 3 of the quick start. `deskhaven key show` prints your old public key.

Device IDs stay the same; clients register again on their own. Users, address books and
the old server's peer list are not carried over.

To sign in and receive strategies, clients also need the API server: import the new
configuration as in [Connect clients](../getting-started/connect-clients.md).

On a [binary deployment](binary-systemd.md), install the keys after creating the
`deskhaven` user and before the first start:

```sh
install -d -o deskhaven -g deskhaven -m 0700 /var/lib/deskhaven
install -o deskhaven -g deskhaven -m 0600 id_ed25519 /var/lib/deskhaven/
install -o deskhaven -g deskhaven -m 0644 id_ed25519.pub /var/lib/deskhaven/
```
