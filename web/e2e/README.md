# Console end-to-end tests

Playwright flows against the real `deskhaven` binary with the embedded console. They
live in `web/` (not `tests/e2e/web/`) so they share the console's lockfile,
TypeScript, ESLint and Prettier setup.

## What runs

`global-setup.ts` starts one `deskhaven serve` per run: a fresh temporary data dir,
client API (and console) on `127.0.0.1:<random>`, the signal server on a free port block,
JSON logs captured to `e2e-results/<backend>/server.log`, and
`DESKHAVEN_SETTINGS__AUTH__TFA__ENFORCE=admins` pinned. It reads the first-run setup link
from the log and stops the server after the run. `DESKHAVEN_SETTINGS__UPDATES__CHECK=false`
is pinned too, so a run never connects to GitHub.

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
trace. CI (`.github/workflows/ci.yml`, job `e2e`) runs the suite once per backend and
uploads that directory when a run fails.

## Remote client codec check

`codec/` holds browser checks of the remote client's media path that need no server, under
their own config. `vp9-decode.codec.ts` bundles `src/features/remote/video/decode-check.ts`
with Vite, loads it into Chromium on an https page (WebCodecs needs a secure context) and
decodes the relay fixture's captured VP9 key frame through the video pipeline.
`opus-decode.codec.ts` encodes a tone into 10 ms Opus packets the way the peer does, clicks to
let the page start an AudioContext, and plays them through the audio pipeline (`audio/`):

```sh
cd web
export PLAYWRIGHT_BROWSERS_PATH=$(nix build --no-link --print-out-paths nixpkgs#playwright-driver.browsers)
export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
nix develop -c pnpm exec playwright test -c e2e/codec/playwright.config.ts
```

`pnpm e2e` does not run it: the console config only matches `*.spec.ts`.

## Remote E2E (real RustDesk clients)

`remote/` drives the browser remote page against official RustDesk clients under Xvfb. It has
its own config and specs named `*.remote.ts`, so `pnpm e2e` and CI's `e2e` job never run it;
it is a manual and nightly check (`.github/workflows/remote-e2e.yml`).

`remote/global-setup.ts` starts, once per run, all with throwaway state: `deskhaven serve`
(relay list pointing at the local hbbr, no public STUN, update check off), the upstream
`hbbr` (`-k` = the server's public key), a same-origin Node proxy (`/ws/relay` to hbbr's
WebSocket port, the rest to the console: the shape of `deploy/compose/Caddyfile`), and one
`rustdesk --server` per client version, each on its own Xvfb with a fixed permanent password.
The owner is created through the setup API, then `login.setup.remote.ts` signs in, turns
`web_client.enabled` on and waits for the devices.

| spec                   | covers                                                                                           |
| ---------------------- | ------------------------------------------------------------------------------------------------ |
| `reconnect.remote.ts`  | relay WebSocket cut under a session: back without the password; past the budget, asked once      |
| `relay.remote.ts`      | (a) device list, "Connect in browser", password, decoded frame, pill "Relay"; 1.4 and 1.5        |
| `webrtc.remote.ts`     | (b) 1.5 over WebRTC; (c) UDP closed (relay-only ICE, no TURN): falls back to the relay           |
| `input.remote.ts`      | (d) click into an xterm, type a random line into `tee`, read the file; Translate and Map         |
| `resolution.remote.ts` | a second Xvfb mode is listed in the Display menu; picking it and "Original" resize the picture   |
| `cursor.remote.ts`     | the peer's shape is the `blob:` CSS cursor under the real CSP; a device-side move is drawn       |
| `clipboard.remote.ts`  | Chromium only: text syncs both ways with no clicks; text copied before the session is not sent   |
| `paste.remote.ts`      | Chromium and Firefox: three paste shortcuts paste the local text into the device's xterm         |
| `keys.remote.ts`       | Input > Send keys: Alt+Tab, PrintScreen and Win show up in a device xterm; 1.4 and 1.5           |
| `viewonly.remote.ts`   | Actions > View only: mouse and keys reach no device (X pointer, xterm) until it is off; 1.4, 1.5 |
| `errors.remote.ts`     | (e) offline device shows `device_offline`; `web_client.enabled` off hides the entry              |
| `zz-release.remote.ts` | closing the tab releases its session (5 connections in a row, limit 4 at once)                   |
| `stability.remote.ts`  | (f) `@slow`: 5-minute session, first-frame time and end-to-end latency once a minute             |
| `perf.remote.ts`       | `@slow`: performance baseline, three network conditions × client × transport (below)             |

Needed on the machine: `Xvfb`, `xterm`, `xclock`, `xdotool`, `xrandr`, `xclip`, `hbbr` (rustdesk-server 1.1.16),
the client launchers, a non-loopback IPv4 address (hbbr treats loopback TCP as its admin
console; override with `RD_E2E_HOST`), Playwright's browsers. On NixOS
`remote/nix-env.sh` fetches pinned ones and prints the exports plus each launcher's store path
and sha256:

```sh
cd web
eval "$(e2e/remote/nix-env.sh)"
export PLAYWRIGHT_BROWSERS_PATH=$(nix build --no-link --print-out-paths nixpkgs#playwright-driver.browsers)
export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
nix develop -c pnpm e2e:remote              # chromium (all but @slow) and firefox (relay, webrtc)
nix develop -c pnpm e2e:remote --project=chromium relay.remote.ts
RD_E2E_SLOW=1 nix develop -c pnpm e2e:remote            # the soak; RD_E2E_SOAK_SECONDS=300
RD_E2E_SLOW=1 nix develop -c pnpm e2e:remote --project=chromium perf.remote.ts  # the baseline
```

Outside nix (a Debian container): install `xvfb xterm x11-apps xdotool x11-xserver-utils xclip`, a RustDesk `.deb`
(`RD_E2E_CLIENT_15=/usr/bin/rustdesk`; only one version can be installed at a time, so run
once per version with the other variable unset), `hbbr` from the rustdesk-server release, and
export `RD_E2E_HBBR`; the client needs `RUSTDESK_FORCED_DISPLAY_SERVER=x11`, which the stack sets.
A client variable that is unset skips that version's tests. Logs of every process are in
`e2e-results/remote/`.

The WebRTC fallback test replaces `RTCPeerConnection` in the page with one that is
relay-only without TURN servers: no candidate is ever gathered, in any browser, while the
attempt itself still happens (the test counts the connections it made).

### Shaped relay link and the performance baseline

`startProxy(console, relay, shaping)` (`remote/support/proxy.ts`) can slow the `/ws/relay`
traffic down, both ways: `rttMs` adds that round trip (half on each direction, bytes kept in
order), `kbps` caps each direction's rate behind a 64 KiB queue, past which the proxy stops
reading so TCP pushes back to the sender. Both are off by default, and then the proxy pipes
the sockets exactly as before. Console traffic is never shaped, and neither is WebRTC, which
does not pass through the proxy.

`perf.remote.ts` chains a shaped proxy in front of the stack's for each test that needs one
and samples one session per case for `RD_E2E_PERF_SECONDS` (default 180): A untouched, B
`rttMs: 80`, C `rttMs: 80, kbps: 2000`; 1.4 and 1.5 over the relay, plus 1.5 over WebRTC in A
only. A and B keep a small xterm printing random bytes on the device screen, C a large one
(`noise` in `remote/support/x11.ts`). It turns the page's statistics overlay on, reads its
`data-stat`/`data-value` cells once a second (fps p50/p05, kbps, decode time, key frame
requests) and probes the end-to-end latency every 7 s the way the soak does (p50, p95, and
the p95 of the first and the last minute). Each test prints one `PERF {json}` line and attaches
the same JSON as `perf.json`; it asserts only that the session stays up and samples came in.
