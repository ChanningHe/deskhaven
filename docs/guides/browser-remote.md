# Control devices from the browser

When you finish, console users open a device from **Devices** (administrators and owners)
or **My devices** (everyone else), type the device's password, and control it in the browser
tab: screen, mouse, keyboard and text clipboard. Nothing is installed on the computer in
front of them.

The browser connects to the device directly over WebRTC when it can, and through your relay
otherwise. It is deskhaven's own client, written for this server; it is not the RustDesk web
client.

## Before you start

- deskhaven is reached over **HTTPS** through a reverse proxy that also forwards
  `/ws/relay` to the relay's WebSocket (21119). The compose deployment and the
  [reverse proxy guide](reverse-proxy.md) already do this.
- `signal.relay_servers` lists your relay, and its **first entry is the relay behind
  `/ws/relay`**. The browser can only reach that one.
- Devices run a RustDesk client that uses this server's key and has no relay server of its
  own in its network settings. Tested with 1.4.3 (relay only) and 1.5.0 (WebRTC and relay).

## Turn it on

Set `web_client.enabled` to `true` in the configuration file and restart deskhaven:

```toml
[settings]
"web_client.enabled" = true
```

or with the environment variable `DESKHAVEN_SETTINGS__WEB_CLIENT__ENABLED=true` (in the compose
deployment, under `environment:` in `compose.override.yaml`). It is off by default; while it is
off the server refuses every browser session and the console shows no **Connect in browser**
action. Only the configuration turns it on: the console shows the setting read-only, and the
admin API refuses to change it.

Who can connect:

| Console user | Devices |
| --- | --- |
| user | the devices they own |
| administrator, owner | every device |

The device still asks for its own password (or for the person at it to accept), exactly as
for a desktop client. Disabled devices cannot be opened. Every session the browser opens is in
the audit log (`web_client.session.open`), with the user, the device, the address it came
from and the result. The server only takes part while the connection is set up, so it does not
know when a session ends: `web_client.session.close` only appears for sessions closed within
that first minute. To turn it off, remove the setting (or set it to `false`) and restart
deskhaven. New sessions are refused from then on; a session already running between the browser
and the device is not ended. A value turned on in **Settings** before this became a
configuration-only setting is ignored, and deskhaven logs a warning about it at every start.

## WebRTC and TURN

The browser tries WebRTC and the relay at the same time and keeps WebRTC if it connects
within a moment of the relay; the bar at the top of the remote screen shows which one won.
WebRTC needs the browser and the device to reach each other over UDP. By default the browser
only knows public STUN servers (`web_client.ice_servers`), which is enough for most home and
office networks; when it is not, the session runs over the relay.

To get WebRTC across stricter networks, run a TURN server and add it:

```text
web_client.ice_servers   = stun:turn.example.com:3478,turn:turn.example.com:3478
web_client.turn_username = deskhaven-web
web_client.turn_credential = <a password for this account alone>
```

The credential is sent to every browser that opens a session, and only to that one TURN
host: all `turn:` / `turns:` URLs in the list must name it. Give it an account of its own.
Set `signal.always_use_relay` to keep every connection, the browser's included, on the relay.

## Separate console address

With the console on a listener of its own ([Keep the console off the internet](separate-console.md)),
set `public_url` to the public HTTPS address: the browser then reaches the relay at
`wss://<public_url host>/ws/relay`, and deskhaven allows that address in the console's
Content-Security-Policy. Without `public_url` the browser looks for `/ws/relay` on the
console's own address.

## Using the remote screen

- A Windows device that shares its RDP sessions and has more than one session asks first which
  one to control, its current session preselected. Choosing another session makes the device
  restart in it and end this connection: reconnect a few seconds later.
- Click the screen to type into it. Press **Esc twice** to leave it with the keyboard.
- **Keyboard mode** (top bar): *Map* sends physical keys and suits most layouts; *Translate*
  and *Legacy* send characters. Input methods (Chinese, Japanese, …) work in every mode.
- **Ctrl+Alt+Del**, the display picker and full screen are in the top bar.
- **Clipboard**: *Send clipboard* puts this computer's clipboard text on the device; the
  device's clipboard text appears under *Device clipboard* for you to copy.
- **AV1** is encoded in software on the device, so on a device with a modest CPU it can deliver
  far fewer frames and bits than VP9 (measured at 1024×768, quality 500: about 19 fps and
  300 kbps with AV1, while VP9 reaches about 8 Mbps). Choose VP9 when smoothness matters.

Not supported yet: audio, file transfer, chat, more than one display at a time.

## When it does not connect

| Message | What to check |
| --- | --- |
| The device is offline | the device is running and registered with this server |
| The device is set to use another relay server | clear the relay server in the device's network settings, or set it to this server's relay |
| No relay server is configured | `signal.relay_servers` |
| The device did not answer | the device's RustDesk service is running; its version |
| Remote control from the browser is turned off | `web_client.enabled` in the configuration file or `DESKHAVEN_SETTINGS__WEB_CLIENT__ENABLED`, then restart |

See also [Known limitations](../reference/known-limitations.md#browser-remote-control).
