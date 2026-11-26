// The device's own log files (a release client logs to ~/.local/share/logs, not to stderr).
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import type { ClientKey, DeviceInfo } from './env.ts'

export function deviceLog(key: ClientKey, info: DeviceInfo): string | null {
  const logs = join(info.scratchDir, `home-${key.replace('.', '')}`, '.local', 'share', 'logs')
  let files: string[]
  try {
    files = readdirSync(logs, { recursive: true, encoding: 'utf8' })
  } catch {
    return null
  }
  const text = files
    .filter((file) => file.endsWith('.log'))
    .map((file) => readFileSync(join(logs, file), 'utf8'))
    .join('\n')
  return text === '' ? null : text
}

export interface QosTarget {
  atMs: number
  fps: number
  /** The reply's delay in ms; absent on a probe-timeout line. */
  delay: number | undefined
  ratio: number | undefined
}

const QOS_TRACE = /qos_trace t=(\d+) [^\n]*/g
const field = (line: string, name: string): number | undefined => {
  const value = new RegExp(`\\b${name}=([\\d.]+)`).exec(line)?.[1]
  return value === undefined ? undefined : Number(value)
}

/** Trace-level lines: only written when the client runs with `RD_E2E_PEER_RUST_LOG`. */
export function qosTargets(log: string): QosTarget[] {
  return [...log.matchAll(QOS_TRACE)].flatMap((m) => {
    const fps = field(m[0], 'fps')
    if (fps === undefined) return []
    return [{ atMs: Number(m[1]), fps, delay: field(m[0], 'delay'), ratio: field(m[0], 'ratio') }]
  })
}
