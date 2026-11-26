import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join } from 'node:path'

import { BINARY_ENV, DEFAULT_BINARY, NO_UPDATE_CHECK } from '../../support/env.ts'
import { freeSignalPort } from '../../support/ports.ts'
import { registerDevice } from '../../support/rendezvous.ts'
import { startServer } from '../../support/server.ts'
import { CLIENT_ENV, DEVICE_PASSWORD, OWNER, relayHost, tool } from './env.ts'
import type { ClientKey, DeviceInfo, StackInfo, TOOL_ENV } from './env.ts'
import { spawnManaged, waitFor } from './procs.ts'
import type { Managed } from './procs.ts'
import { startProxy } from './proxy.ts'

const SETUP_HEADER = { 'Content-Type': 'application/json', 'X-Deskhaven-CSRF': 'anonymous' }
// `-wr`: a white root window, so a desktop with nothing on it is not an all-black frame.
const SCREEN = '1280x800x24'
const FIRST_DISPLAY = 90

export interface Stack {
  info: StackInfo
  stop: () => Promise<void>
}

async function listening(port: number): Promise<boolean> {
  const { connect } = await import('node:net')
  return new Promise((resolve) => {
    const socket = connect(port, '127.0.0.1')
    socket.once('connect', () => {
      socket.destroy()
      resolve(true)
    })
    socket.once('error', () => {
      resolve(false)
    })
  })
}

function freeDisplay(taken: Set<number>): number {
  for (let n = FIRST_DISPLAY; n < FIRST_DISPLAY + 100; n += 1) {
    if (taken.has(n)) continue
    if (!existsSync(`/tmp/.X${String(n)}-lock`) && !existsSync(`/tmp/.X11-unix/X${String(n)}`)) {
      taken.add(n)
      return n
    }
  }
  throw new Error('no free X display')
}

function clientVersion(launcher: string): string {
  const reply = spawnSync(launcher, ['--version'], { encoding: 'utf8', timeout: 20_000 })
  return `${reply.stdout}${reply.stderr}`.trim().split('\n').pop() ?? 'unknown'
}

interface ClientContext {
  root: string
  logs: string
  serverPublicKey: string
  signalPort: number
  relayAddr: string
  displays: Set<number>
  managed: Managed[]
}

async function startClient(
  ctx: ClientContext,
  key: ClientKey,
  launcher: string,
): Promise<DeviceInfo> {
  const name = key.replace('.', '')
  const home = join(ctx.root, `home-${name}`)
  const config = join(home, '.config', 'rustdesk')
  await mkdir(config, { recursive: true })
  const id = `9${String(Math.floor(Math.random() * 1e8)).padStart(8, '0')}`
  await writeFile(
    join(config, 'RustDesk2.toml'),
    [
      `rendezvous_server = '127.0.0.1:${String(ctx.signalPort)}'`,
      'nat_type = 1',
      'serial = 0',
      '',
      '[options]',
      `custom-rendezvous-server = '127.0.0.1:${String(ctx.signalPort)}'`,
      `relay-server = '${ctx.relayAddr}'`,
      `key = '${ctx.serverPublicKey}'`,
      "verification-method = 'use-permanent-password'",
      "approve-mode = 'password'",
      '',
    ].join('\n'),
  )
  // The client encrypts a plain-text password on its first start.
  await writeFile(join(config, 'RustDesk.toml'), `id = '${id}'\npassword = '${DEVICE_PASSWORD}'\n`)

  const display = `:${String(freeDisplay(ctx.displays))}`
  const base = { PATH: process.env.PATH ?? '', HOME: home, XDG_CONFIG_HOME: join(home, '.config') }
  ctx.managed.push(
    spawnManaged(
      'xvfb-' + name,
      tool('xvfb'),
      [display, '-screen', '0', SCREEN, '-nolisten', 'tcp', '-wr'],
      {
        env: base,
        logFile: join(ctx.logs, `xvfb-${name}.log`),
      },
    ),
  )
  await waitFor(
    `X display ${display}`,
    () => existsSync(`/tmp/.X11-unix/X${display.slice(1)}`),
    20_000,
  )
  const x = { ...base, DISPLAY: display }
  // Without a logind seat the client reports "tty" and refuses logins.
  ctx.managed.push(
    spawnManaged('rustdesk-' + name, launcher, ['--server'], {
      env: {
        ...x,
        PATH: withToolDir(x.PATH, 'xrandr'),
        ...xdoLibrary(),
        RUSTDESK_FORCED_DISPLAY_SERVER: 'x11',
        XDG_SESSION_TYPE: 'x11',
        ...peerLogFilter(),
      },
      logFile: join(ctx.logs, `rustdesk-${name}.log`),
    }),
  )
  return { id, display, scratchDir: ctx.root }
}

/** The client's RUST_LOG, e.g. `debug,librustdesk::server::video_qos=trace` for perf.remote.ts. */
function peerLogFilter(): { RUST_LOG?: string } {
  const filter = process.env.RD_E2E_PEER_RUST_LOG
  return filter === undefined || filter === '' ? {} : { RUST_LOG: filter }
}

// nixpkgs' 1.5 cannot dlopen libxdo, and without it sends no CursorPosition.
function xdoLibrary(): { LD_LIBRARY_PATH?: string } {
  const file = tool('xdotool')
  if (!isAbsolute(file)) return {}
  const lib = join(dirname(dirname(file)), 'lib')
  return existsSync(lib) ? { LD_LIBRARY_PATH: lib } : {}
}

/** The client lists and changes resolutions by running `xrandr`, which it looks up on PATH. */
function withToolDir(path: string, name: keyof typeof TOOL_ENV): string {
  const file = tool(name)
  return isAbsolute(file) ? `${dirname(file)}:${path}` : path
}

/** Through the setup wizard's API: its UI steps are decoration. */
async function createOwner(baseUrl: string, setupPath: string) {
  const token = new URLSearchParams(setupPath.split('#')[1]).get('token')
  const reply = await fetch(new URL('/admin/api/v1/setup', baseUrl), {
    method: 'POST',
    headers: SETUP_HEADER,
    body: JSON.stringify({ token, username: OWNER.username, password: OWNER.password }),
  })
  if (!reply.ok)
    throw new Error(`owner setup failed: ${String(reply.status)} ${await reply.text()}`)
}

export async function startStack(resultsDir: string): Promise<Stack> {
  const binary = process.env[BINARY_ENV] ?? DEFAULT_BINARY
  if (!existsSync(binary)) throw new Error(`${binary} not found: build it or set ${BINARY_ENV}`)
  const launchers = (Object.keys(CLIENT_ENV) as ClientKey[]).flatMap((key) => {
    const path = process.env[CLIENT_ENV[key]]
    return path === undefined || path === '' ? [] : ([[key, path]] as const)
  })
  if (launchers.length === 0) {
    throw new Error(`set ${Object.values(CLIENT_ENV).join(' and/or ')} to a RustDesk launcher`)
  }
  await mkdir(resultsDir, { recursive: true })
  const root = await mkdtemp(join(tmpdir(), 'deskhaven-remote-e2e-'))
  const managed: Managed[] = []
  let server: Awaited<ReturnType<typeof startServer>> | undefined
  let proxy: Awaited<ReturnType<typeof startProxy>> | undefined
  const stop = async () => {
    for (const child of managed.reverse()) await child.stop()
    await proxy?.stop()
    await server?.stop()
    await rm(root, { recursive: true, force: true })
  }
  try {
    const signalPort = await freeSignalPort()
    // The block of `freeSignalPort` spans port-1..port+2; the relay sits beyond it, inside the
    // ten ports reserved per block.
    const relayPort = signalPort + 5
    const relayAddr = `${relayHost()}:${String(relayPort)}`
    const dataDir = join(root, 'data')
    await mkdir(dataDir, { mode: 0o700 })
    server = await startServer({
      binary,
      dataDir,
      logFile: join(resultsDir, 'server.log'),
      signalPort,
      env: {
        [NO_UPDATE_CHECK.env]: NO_UPDATE_CHECK.value,
        DESKHAVEN_SETTINGS__SIGNAL__RELAY_SERVERS: relayAddr,
        DESKHAVEN_SETTINGS__WEB_CLIENT__ENABLED: 'true',
        // No public STUN: the run needs no outbound network, host candidates connect.
        DESKHAVEN_SETTINGS__WEB_CLIENT__ICE_SERVERS: process.env.RD_E2E_ICE_SERVERS ?? '',
      },
    })
    await createOwner(server.baseUrl, server.setupPath)

    const shown = spawnSync(binary, ['key', 'show', '--data-dir', dataDir], { encoding: 'utf8' })
    const serverPublicKey = shown.stdout.split('\n')[0]?.trim() ?? ''
    if (shown.status !== 0 || serverPublicKey === '') throw new Error('cannot read the server key')
    await mkdir(join(root, 'hbbr'))
    managed.push(
      spawnManaged('hbbr', tool('hbbr'), ['-k', serverPublicKey, '-p', String(relayPort)], {
        env: { PATH: process.env.PATH ?? '' },
        cwd: join(root, 'hbbr'),
        logFile: join(resultsDir, 'hbbr.log'),
      }),
    )
    await waitFor('hbbr', () => listening(relayPort + 2), 20_000)
    const consoleUrl = new URL(server.baseUrl)
    proxy = await startProxy(
      { host: consoleUrl.hostname, port: Number(consoleUrl.port) },
      { host: '127.0.0.1', port: relayPort + 2 },
    )

    const ctx: ClientContext = {
      root,
      logs: resultsDir,
      serverPublicKey,
      signalPort,
      relayAddr,
      displays: new Set(),
      managed,
    }
    const devices: StackInfo['devices'] = {}
    const clientVersions: Record<string, string> = {}
    for (const [key, launcher] of launchers) {
      devices[key] = await startClient(ctx, key, launcher)
      clientVersions[key] = `${clientVersion(launcher)} (${launcher})`
    }
    // Registered once and never again: offline after the signal server's peer timeout.
    const ghostId = `8${String(Math.floor(Math.random() * 1e8)).padStart(8, '0')}`
    await registerDevice(signalPort, { rustdeskId: ghostId, uuid: Buffer.alloc(16, 9) })

    return {
      info: { baseUrl: proxy.origin, devices, ghostId, clientVersions },
      stop,
    }
  } catch (error) {
    await stop()
    throw error
  }
}
