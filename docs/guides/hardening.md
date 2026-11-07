# Harden your server

When you finish, your server is tightened beyond the defaults for life on the internet.

1. **Back up the master key apart from the database** and move it out of the data volume
   into a secret file ([how](backup-upgrade.md#keep-the-master-key-apart)): a backup that
   holds both gives away every stored secret.
2. **Require two-factor authentication**: **Identity → Require two-factor authentication
   for**, at least **Administrators and owners**. A covered user enrols in the console
   before the RustDesk client lets them sign in. OIDC sign-ins are exempt, also into local
   accounts with a [linked sign-in](oidc.md#link-an-existing-account); require MFA at the
   identity provider.
3. **Keep the console off the internet**: [give it its own port](separate-console.md).
4. **Refuse the old key exchange** once all your clients support version 1 of it, that is
   RustDesk 1.5.0 or later: set `signal.require_kx_v1` to `true` under **Settings →
   Signalling**. Version 0 uses one key for both directions. Signed-in 1.4.x clients only
   speak version 0 and are then refused, so it stays off by default.
5. **Trust only your proxy**: `security.trusted_proxies` should name exactly the address
   your reverse proxy connects from. The compose deployment sets it for you.
6. **Firewall the local ports** on a binary deployment: 21119 and the relay's command
   console ([step 3](binary-systemd.md#3-firewall-the-local-ports)). Keep 21118 on
   loopback with `signal.ws_listen = "127.0.0.1:21118"`; the systemd unit sets it.
7. **Give OIDC an admin group only** when the identity provider's administrators alone
   control that group ([OIDC](oidc.md#groups-and-admins)).

8. **Keep `auth.oidc.link_email_max_role` at `user` or `off`** unless the identity provider
   does not let its users change their own email address ([OIDC](oidc.md#link-by-email)).

## Recover a compromised account

Resetting the password, resetting two-factor authentication and revoking sessions leave the
account's linked OIDC sign-ins in place, and those sign in without a password or second
factor. Open the user under **Users**, check **Linked sign-ins**, and **Unlink** every one
you do not trust. Then reset the password and two-factor authentication, and revoke the
sessions.

What remains open is listed in [Known limitations](../reference/known-limitations.md).
