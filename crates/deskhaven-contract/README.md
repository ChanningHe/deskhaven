# deskhaven-contract

Contract tests for the client API (`/api/*`): does the **official RustDesk
client** accept what deskhaven answers? (dev/docs/09-testing.md §2,
spec: dev/docs/03-client-api-contract.md.)

- `src/mirror/`: client-mirror types, i.e. what the client deserializes, each
  citing the client source `file:line`. `Deserialize` only.
- `tests/contract/fixtures/<endpoint>/<case>.json` (repository root): one
  request + expectations per file.
- `tests/contract/UPSTREAM_COMMIT`: the rustdesk commit all fixtures were
  verified against.
- The runner replays each fixture through the **real** `client_api` router and
  middleware stack (`deskhaven::http::apps` + `deskhaven_http::apply_middleware`)
  over a fresh temporary data_dir and SQLite database (migrated, default
  settings), via `tower::ServiceExt::oneshot`.

```sh
nix develop -c cargo test -p deskhaven-contract                     # everything
CONTRACT_FILTER=login-options/ nix develop -c cargo test -p deskhaven-contract --test contract -- --nocapture
```

## Adding a fixture

1. Read the client code that sends the request and parses the reply, at the
   commit in `tests/contract/UPSTREAM_COMMIT`. Note every `file:line` you rely on.
2. Create `tests/contract/fixtures/<endpoint>/<case>.json`:

   ```json
   {
     "upstream_commit": "23e5bb471a040d62f0a6ad52591fb30784265b73",
     "source": ["flutter/lib/models/user_model.dart:240-264"],
     "description": "What the client does with this reply, and why.",
     "seed": [],
     "request": {
       "method": "GET",
       "path": "/api/login-options",
       "headers": {},
       "body": "empty"
     },
     "expect": {
       "status": 200,
       "headers": {},
       "body_kind": "json",
       "shape": "LoginOptions",
       "json_subset": [],
       "json_absent": []
     }
   }
   ```

3. If the reply needs a type the client parses and `src/shapes.rs` does not
   list yet: add a mirror type under `src/mirror/` (doc comment with
   `file:line`, the client's optionality and types, unknown fields ignored)
   with unit tests from the 03 example, then register its name in `SHAPES`.
4. If the request needs prior state (user, session, device, settings): add a
   seed to `src/seeds.rs` that writes it through the real services of the
   app's `AppState` (the same instance the handlers serve) and list its name
   under `"seed"`. A seed returns named outputs (e.g. `session_token` → the
   token the real login service issued); refer to them as `${session_token}`
   in `request.path`, header values and body strings. Seeds run in order; an
   output name defined twice, or a placeholder without an output, fails the
   fixture. There is no escape for a literal `${`. Built-in seeds (every
   account's password is `contract-Seed-pass-1`):
   - `client-session`: active local user `alice` logged in from device
     `123456789` / `contract-seed-device-uuid`; outputs `session_token`,
     `username`, `rustdesk_id`, `uuid`.
   - `local-user`: active local user `alice`, not logged in.
   - `disabled-user` / `unverified-user`: `dave` (status disabled) / `erin`
     (status unverified).
   - `console-session`: active user `carol` with a console session; outputs
     `console_token`.
   - `login-locked-out`: spends the whole `auth.lockout.max_failures` budget
     of `alice` and of the unknown `nobody` from the default peer, so their
     next login from there is locked out.
   - `totp-user`: active user `alice` with TOTP enabled (confirmed with the
     previous step's code, so the current and next codes still log in);
     outputs `totp_secret` (base32) and `recovery_code`.
   - `tfa-enforce-all`: sets `auth.tfa.enforce` to `all`.
   - Address books (after `client-session`; `src/ab_seeds.rs`):
     `ab-personal` (alice's personal book with peer `111111111` and tag
     `work`; outputs `personal_guid`), `ab-shared-read` / `ab-shared-write` /
     `ab-shared-full` (admin-managed `Team Read` / `Team Write` / `Team Full`
     granted to alice at rule 1 / 2 / 3, each with peer `222222222` and tag
     `ops`; outputs `read_guid` / `write_guid` / `full_guid`), `ab-foreign`
     (user `bob` with a personal and an owned shared book; outputs
     `foreign_personal_guid`, `foreign_shared_guid`), `ab-one-peer-max`
     (`addressbook.max_peers` = 1).
5. Run the contract test. Watch it fail before the endpoint exists (red),
   then implement the endpoint until it passes.

### Format

| key | meaning |
|---|---|
| `listen` | optional topology the request is sent to: `"client_api"` (default, the client API router alone), `"single"` (client API + console on one listener) or `{"split": {"admin_public_url": "https://..."}}` (the client API listener of a split deployment: `GET /` redirects). A `single` fixture cannot pin the status of `/` (200 with a built `web/dist`, 503 without): use a Rust test (`tests/console_root.rs`) |
| `request.method`, `request.path` | path includes the query string as sent (`/api/ab/peers?current=1&pageSize=100&ab=g1`) |
| `request.headers` | sent exactly as written; no implicit `content-type` (Dart's `http.post` with a `String` body sends `text/plain; charset=utf-8`, the Rust client `application/json`; copy what the client sends). `content-length` is added from the body when a body or a non-GET/HEAD method is used (empty POSTs carry `Content-Length: 0`, 03 §1.3) |
| `request.body` | `"empty"` (default), `{"json": <value>}`, `{"text": "..."}` |
| `request.peer` | TCP peer, default `203.0.113.7:50000` |
| `expect.status` | exact status |
| `expect.headers` | response headers by name (names in any case): `"exact value"` or `{"contains": "..."}`; values are case-sensitive, a repeated header is joined with `", "`, a missing one fails (`{"Allow": "GET,HEAD", "content-type": "application/json"}`) |
| `expect.body_kind` | `empty` (zero bytes), `json`, `text` (UTF-8) |
| `expect.shape` | (json) mirror type name from `src/shapes.rs` that must parse the raw bytes |
| `expect.json_subset` | (json) objects: listed keys match, others ignored; arrays: same length, element-wise; scalars: equal, numbers by value (`1` == `1.0`; shape checks still see the exact type) |
| `expect.json_absent` | (json) JSON pointers that must not exist (`/strategy`) |
| `expect.text` | (text) exact body |

Unknown keys are errors, so a typo cannot silently drop an assertion.

## Multi-step flows

A fixture is one request. Flows whose meaning is the *sequence* (logout then
401, the 2FA two-step login, OIDC polling, heartbeat nonce dedupe, expiry) are
Rust tests on `TestApp`, decoding each reply with the mirror types, as the
client would. `tests/flows.rs` is the template:

```rust
#[tokio::test]
async fn login_options_still_answers_after_a_rejected_method() {
    let app = TestApp::start().await.unwrap(); // one database, one AppState

    let rejected = app.send(&Request::new("POST", "/api/login-options")).await.unwrap();
    let options = app.send(&Request::new("GET", "/api/login-options")).await.unwrap();

    assert_eq!(rejected.status, 405);
    assert_eq!(rejected.json::<ErrorBody>().unwrap().error, "method not allowed");
    assert!(options.json::<LoginOptions>().unwrap().is_empty());
}
```

Prepare state with the same seeds (`Seeds::builtin().apply(&names,
&app.state)`), directly through `app.state`'s services, or with
`app.set_setting(key, value)` (hot-reloaded runtime settings);
`TestApp::start_with_options(TestOptions { routes, hasher, trusted_proxies, public_url, .. })`
adds test routes, rebuilds users and sessions around a given password hasher
(e.g. one slot, to provoke `Busy`), trusts proxy ranges or sets `public_url` (OIDC). `requests::{login,
second_step, bearer, oidc_auth, oidc_query, oidc_callback, from}` build the requests (the 2FA second step
included) exactly as the client sends them. Set
`Request::headers`/`body` for the client's exact request, and carry values
from one reply into the next request in plain Rust.

Time: there is no clock control yet, so expiry cannot be tested without
waiting. M2-05 introduces an injectable clock; expiry tests start there.

## Provenance rules

- `source` is mandatory and every entry is `path:line` or `path:start-end`,
  relative to the rustdesk repository. Cite the code that *parses* the reply
  (and sends the request), not deskhaven code or our docs.
- `upstream_commit` must equal `tests/contract/UPSTREAM_COMMIT`. When the
  weekly tracking job (09 §2.3) reports a client change: re-read the cited
  lines at the new commit, update 03, the mirror types and the fixtures, and
  only then bump `UPSTREAM_COMMIT` (every fixture fails until it is re-verified).
- Expectations come from the client source, never from what the server
  currently answers.

## Never import this crate in production code

Mirror types exist so the server's responses are checked by an independent
description of the client. If production code serialized with them, the test
would only prove that serde round-trips. Therefore:

- deskhaven-contract may only be a dev-dependency:
  `tests/no_production_dependents.rs` asks `cargo metadata --no-deps` and
  fails if any workspace package depends on it with a kind other than `dev`
  (normal or build, any `[target.*]`, any rename, inherited or not: Cargo has
  already resolved all of that). For every crate the server depends on, Cargo
  would reject it anyway (this crate depends on `deskhaven`, so it would be a
  cycle).
- Mirror types are `Deserialize` only: they cannot produce a response.
