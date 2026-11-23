// What the remote stack needs from the machine (tool paths), and what its global setup
// hands to the specs (through the environment, which Playwright passes on to its workers).
import { networkInterfaces } from 'node:os'

/** The official client launchers under test; a project whose variable is unset is skipped. */
export const CLIENT_ENV = {
  '1.4': 'RD_E2E_CLIENT_14',
  '1.5': 'RD_E2E_CLIENT_15',
} as const
export type ClientKey = keyof typeof CLIENT_ENV

export const TOOL_ENV = {
  hbbr: ['RD_E2E_HBBR', 'hbbr'],
  xvfb: ['RD_E2E_XVFB', 'Xvfb'],
  xterm: ['RD_E2E_XTERM', 'xterm'],
  xclock: ['RD_E2E_XCLOCK', 'xclock'],
  xdotool: ['RD_E2E_XDOTOOL', 'xdotool'],
  xrandr: ['RD_E2E_XRANDR', 'xrandr'],
  xclip: ['RD_E2E_XCLIP', 'xclip'],
} as const

/** JSON of `StackInfo`, set by the global setup. */
export const STACK_ENV = 'E2E_REMOTE_STACK'
export const BASE_URL_ENV = 'E2E_BASE_URL'

/** The device password every client in the stack is given. */
export const DEVICE_PASSWORD = 'Remote-E2e-7f3a91'

export const OWNER = { username: 'owner', password: 'e2e owner passphrase 1' } as const

export interface DeviceInfo {
  /** The RustDesk ID the console lists. */
  id: string
  /** The X display of the device, `:N`. */
  display: string
  /** The temp dir of the run, where a test's xterm shell starts. */
  scratchDir: string
}

export interface StackInfo {
  baseUrl: string
  /** Client version key -> its device; only the clients that were started. */
  devices: Partial<Record<ClientKey, DeviceInfo>>
  /** A registered device that never answers and is offline soon after the setup. */
  ghostId: string
  clientVersions: Record<string, string>
}

export function tool(name: keyof typeof TOOL_ENV): string {
  const [variable, fallback] = TOOL_ENV[name]
  return process.env[variable] ?? fallback
}

export function stackInfo(): StackInfo {
  const raw = process.env[STACK_ENV]
  if (raw === undefined || raw === '') throw new Error(`${STACK_ENV} is not set (global setup)`)
  return JSON.parse(raw) as StackInfo
}

/**
 * An address of this machine that is not loopback: hbbr takes loopback TCP connections for
 * its admin console, so a device must reach it on another address.
 */
export function relayHost(): string {
  const forced = process.env.RD_E2E_HOST
  if (forced !== undefined && forced !== '') return forced
  for (const list of Object.values(networkInterfaces())) {
    for (const address of list ?? []) {
      if (address.family === 'IPv4' && !address.internal) return address.address
    }
  }
  throw new Error('no non-loopback IPv4 address; set RD_E2E_HOST')
}
