# Sign in with LDAP or Active Directory

When you finish, users sign in to the RustDesk client and the console with their directory
username and password.

Only the owner can configure the directory.

## 1. Enter the directory

In the console, open **Identity**, and under **LDAP directory** fill in the form. For Active
Directory:

| Field | Value |
| --- | --- |
| URL | `ldaps://dc1.corp.example.com:636` |
| Bind DN | `CN=deskhaven,OU=Service Accounts,DC=corp,DC=example,DC=com` |
| Bind password | the service account's password |
| User base DN | `DC=corp,DC=example,DC=com` |
| User filter | `(&(objectClass=user)(objectCategory=person))` |
| Username attribute | `sAMAccountName` |
| Email attribute | `mail` |
| Display name attribute | `displayName` |
| Admin group DN | `CN=RustDesk Admins,OU=Groups,DC=corp,DC=example,DC=com` |

For OpenLDAP, leave the user filter and username attribute blank (`(objectClass=person)`
and `uid`).

Choose **Save directory**.

## 2. Test it

Under **Test the directory**, choose **Test connection**, then **Test user** with a
directory username. Each step shows its result.

## How accounts work

- An account is created at the user's first sign-in.
- Role and groups are synced at every sign-in. Members of the admin group become
  administrators, everyone else a user; owners stay owners. With **Group base DN** set, the
  account is a member of exactly the deskhaven user groups named like its directory groups
  below that DN. deskhaven creates no groups; create them in the console.
- Directory accounts have no local password. When the directory is unreachable, they cannot
  sign in.

The directory's certificate is checked against the operating system's CA store; see
[Known limitations](../reference/known-limitations.md#accounts-and-sign-in).
