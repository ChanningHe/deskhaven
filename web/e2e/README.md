# Console end-to-end tests

Playwright flows against the real `deskhaven` binary with the embedded console. They
live in `web/` (not `tests/e2e/web/`) so they share the console's lockfile,
TypeScript, ESLint and Prettier setup.

## What runs

`global-setup.ts` starts one `deskhaven serve` per run: a fresh temporary data dir,
client API (and console) on `127.0.0.1:<random>`, the signal server on a free port block,
JSON logs captured to `e2e-results/<backend>/server.log`, and
`DESKHAVEN_SETTINGS__AUTH__TFA__ENFORCE=admins` pinned. It reads the first-run setup link
from the log and stops the server after the run.

| project     | spec                   | flow                                                                                                                   |
| ----------- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `first-run` | `first-run.spec.ts`    | setup link → owner → forced TOTP enrolment → sign out → sign in with the code; the setup link works only once          |
| `console`   | `users.spec.ts`        | new user → user group → client login → revoke their sessions → the client token gets 401                               |
|             | `strategies.spec.ts`   | device registers (UDP) → device group → strategy → assigned to the group → preview merges it → heartbeat delivers it   |
|             | `settings.spec.ts`     | saved setting shows "Modified"; the env-pinned setting is read-only and names its variable; concurrent save → conflict |
|             | `self-service.spec.ts` | a user binds TOTP on My account → the client login asks for the code                                                   |

`console` depends on `first-run` and reuses the owner's session (`e2e/.auth/owner.json`).
The flows build on one server, so they run in one worker, in order.

## Running locally

Build the console, then the binary (it embeds `web/dist` at compile time):

```sh
nix develop -c pnpm -C web build
nix develop -c cargo build -p deskhaven
```

Playwright's own browser downloads do not run on NixOS; use nixpkgs' browsers. Their
version must match `@playwright/test` in `package.json`
(`nix path-info nixpkgs#playwright-driver` shows it):

```sh
cd web
export PLAYWRIGHT_BROWSERS_PATH=$(nix build --no-link --print-out-paths nixpkgs#playwright-driver.browsers)
export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
nix develop -c pnpm e2e
```

PostgreSQL instead of SQLite: point `DESKHAVEN_E2E_PG_URL` at an **empty** database (the
first-run flow needs an instance without an owner; startup fails otherwise):

```sh
DESKHAVEN_E2E_PG_URL=postgres://deskhaven@127.0.0.1:5432/deskhaven_e2e nix develop -c pnpm e2e
```

`DESKHAVEN_E2E_BIN` selects another binary (default: `target/debug/deskhaven`).

Reports, traces of failed tests and the server log land in
`e2e-results/{sqlite,postgres}/`; `pnpm exec playwright show-trace <trace.zip>` opens a
trace. CI (`.forgejo/workflows/ci.yml`, job `e2e`) runs the suite once per backend and
uploads that directory when a run fails.
