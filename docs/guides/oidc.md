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

## Existing accounts

A provider identity joins an existing account only when both have the same verified email.
Entra ID sends no `email_verified`, so it never joins existing accounts. See
[Known limitations](../reference/known-limitations.md#accounts-and-sign-in).
