# Sign in with OIDC

When you finish, users sign in to the RustDesk client and the console with an identity
provider such as Keycloak, Authentik, Google or Microsoft Entra ID.

Only the owner can add a provider.

## 1. Register deskhaven at the provider

In the console, open **Identity**. Copy the **Callback URL** under **OIDC providers**
(`https://rd.example.com/api/oidc/callback`). Register a web application (a confidential
client) at your provider with that redirect URI, and note its issuer, client ID and client
secret.

| Provider | Where | Issuer URL |
| --- | --- | --- |
| Keycloak | **Clients → Create client**, OpenID Connect, client authentication on | `https://keycloak.example.com/realms/<realm>` |
| Authentik | **Applications → Create with provider**, OAuth2/OpenID | `https://authentik.example.com/application/o/<slug>/` |
| Google | Google Cloud console, **APIs & Services → Credentials → OAuth client ID**, web application | `https://accounts.google.com` |
| Entra ID | **App registrations → New registration**, platform Web; a secret under **Certificates & secrets** | `https://login.microsoftonline.com/<tenant-id>/v2.0` |
| Pocket ID | **OIDC Clients → Add OIDC Client**, public client off, callback URL from the console | its `APP_URL`, no trailing slash: `https://id.example.com` |

The issuer must match the provider's own `issuer` exactly, trailing slash included. It
cannot be changed later.

## 2. Add the provider

Under **OIDC providers**, choose **New provider** and enter:

- **Name**: shown on the client's sign-in button, e.g. `keycloak`.
- **Issuer URL**, **Client ID**, **Client secret** from step 1.
- **Create an account for unknown users**: on, unless you create every account yourself.

Choose **Create provider**. The console sign-in page and the RustDesk client now offer it.

## Groups and admins

Set **Group claim** to the ID token claim that lists the user's groups, usually `groups`.
A new account joins the existing deskhaven user groups of the same names, and becomes an
administrator if it is in the **Admin group**. Both apply once, when the account is
created; later changes are made in the console. Set **Admin group** only when the
provider's administrators alone decide who is in that group.

- Keycloak: add a **Group Membership** mapper to the client's dedicated scope, token claim
  name `groups`, full group path off.
- Authentik: the default `profile` scope already sends `groups`.
- Entra ID: **Token configuration → Add groups claim**. It sends group object IDs, so the
  admin group is an object ID.
- Pocket ID: sends `groups` only when asked for it. Set the provider's **Scopes** to
  `openid profile email groups`.

## Link an existing account

Someone who already has a deskhaven account links the provider to it once:

1. Sign in to the console with the account's password.
2. Open **My account → Linked sign-ins** and choose **Link** next to the provider.
3. Enter the password, and the two-factor code if the account has one.
4. Sign in at the provider, then confirm **Link** on the page that names the provider and
   the account.

From then on the account signs in through the provider in the console and in the RustDesk
client, without its two-factor code: the account is as safe as its provider account.
**Unlink** removes the link (password required). Administrators see the linked sign-ins
in a user's details and can unlink them, but cannot link for someone. LDAP accounts cannot
link.

## Link by email

A provider login also joins an existing account by itself when all of these hold:

- the provider says the address is verified (`email_verified`; Entra ID never does);
- the account's address is the same and verified (an administrator marks it verified in
  the user's details, after checking it);
- the account has no two-factor authentication and is not an LDAP account;
- the account's role is at most `auth.oidc.link_email_max_role` (default `user`; `off`
  turns this off).

These conditions are checked again at every login through an identity linked this way. If
one fails later, that login is refused and the user links the account as above.

Unlinking any linked sign-in, however it was linked, also marks the account's email
unverified; otherwise the same provider account would link again by email at its next
login. The account's other sign-ins that were linked by email then stop working at their
next login. To allow linking by email again, check the address and mark it verified in the
user's details; the user can also link sign-ins themselves.

Every link by email and every account the provider creates is recorded in the console audit
log, with the system as the actor.

> **Warning:** set `auth.oidc.link_email_max_role` to `admin` or `owner` only when the
> provider does not let its users change their own email address. Otherwise anyone who can
> set an administrator's address at the provider signs in as that administrator.

With **Create an account for unknown users** on, an address that belongs to an account
that may not be joined is refused; no second account is created. See
[Known limitations](../reference/known-limitations.md#accounts-and-sign-in).
