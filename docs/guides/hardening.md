# Harden your server

When you finish, the defaults are tightened for a server on the internet. The defaults are
already safe; each step below closes a further gap.

1. **Back up the master key.** Without it the encrypted secrets are gone. See
   [Back up and upgrade](backup-upgrade.md).
2. **Require two-factor authentication** for administrators: **Identity → Require
   two-factor authentication for → Administrators and owners**. Accounts that sign in with
   OIDC are exempt; require MFA at the identity provider.
3. **Keep the console off the internet**: [give it its own port](separate-console.md).
4. **Refuse the old key exchange** once all your clients support version 1 of it: set
   `signal.require_kx_v1` to `true` under **Settings → Signalling**. Clients that only
   speak version 0 are then refused.
5. **Trust only your proxy**: `security.trusted_proxies` should name exactly the address
   your reverse proxy connects from. The compose deployment sets it for you.
6. **Firewall the local ports** on a binary deployment: 21118, 21119 and the relay's
   command console ([step 3](binary-systemd.md#3-firewall-the-local-ports)).
7. **Leave the device checks on**: keep `devices.verify_uuid` on and
   `devices.accept_presets` at `off`.
8. **Give OIDC an admin group only** when the identity provider's administrators alone
   control that group ([OIDC](oidc.md#groups-and-admins)).

What remains open is listed in [Known limitations](../reference/known-limitations.md).
