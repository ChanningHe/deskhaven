// The deskhaven binary under test: a fresh data dir, loopback listeners, JSON logs on
// stderr. Startup is over when the log says "ready"; the first-run setup link and the
// client API address are read from the same log.
import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { createInterface } from 'node:readline'

const STARTUP_TIMEOUT_MS = 60_000
const SHUTDOWN_TIMEOUT_MS = 15_000

export interface ServerOptions {
  binary: string
  dataDir: string
  logFile: string
  signalPort: number
  /** Extra `DESKHAVEN_*` variables (database URL, pinned settings). */
  env: Record<string, string>
}

export interface RunningServer {
  baseUrl: string
  /** `/setup#token=…` as the server logged it. */
  setupPath: string
  stop: () => Promise<void>
}

/** The `fields` of one JSON log line, or `undefined` for anything else. */
function logFields(line: string): Record<string, unknown> | undefined {
  try {
    const event: unknown = JSON.parse(line)
    if (typeof event !== 'object' || event === null || !('fields' in event)) return undefined
    const fields = event.fields
    return typeof fields === 'object' && fields !== null
      ? (fields as Record<string, unknown>)
      : undefined
  } catch {
    return undefined
  }
}

function serverEnv(options: ServerOptions): NodeJS.ProcessEnv {
  return {
    PATH: process.env.PATH ?? '',
    DESKHAVEN_DATA_DIR: options.dataDir,
    DESKHAVEN_LISTEN__CLIENT_API: '127.0.0.1:0',
    DESKHAVEN_SIGNAL__PORT: String(options.signalPort),
    DESKHAVEN_LOG__FORMAT: 'json',
    DESKHAVEN_LOG__LEVEL: 'info',
    ...options.env,
  }
}

function stopper(child: ChildProcess): () => Promise<void> {
  return () =>
    new Promise((resolve, reject) => {
      if (child.exitCode !== null || child.signalCode !== null) {
        resolve()
        return
      }
      const timer = setTimeout(() => {
        child.kill('SIGKILL')
        reject(new Error(`deskhaven did not stop within ${SHUTDOWN_TIMEOUT_MS} ms`))
      }, SHUTDOWN_TIMEOUT_MS)
      child.once('exit', () => {
        clearTimeout(timer)
        resolve()
      })
      child.kill('SIGTERM')
    })
}

/** Starts `deskhaven serve` and resolves once it is ready, with what the tests need. */
export function startServer(options: ServerOptions): Promise<RunningServer> {
  const child = spawn(options.binary, ['serve'], {
    cwd: options.dataDir,
    env: serverEnv(options),
    stdio: ['ignore', 'ignore', 'pipe'],
  })
  const log = createWriteStream(options.logFile)
  const stop = stopper(child)
  let addr: string | undefined
  let setupPath: string | undefined

  return new Promise((resolve, reject) => {
    const fail = (reason: string) => {
      child.kill('SIGKILL')
      reject(new Error(`${reason}; server log: ${options.logFile}`))
    }
    const timer = setTimeout(() => {
      fail(`deskhaven was not ready within ${STARTUP_TIMEOUT_MS} ms`)
    }, STARTUP_TIMEOUT_MS)
    child.once('error', (error) => {
      clearTimeout(timer)
      fail(`cannot start ${options.binary}: ${error.message}`)
    })
    child.once('exit', (code, signal) => {
      clearTimeout(timer)
      fail(`deskhaven exited during startup (code ${String(code)}, signal ${String(signal)})`)
    })
    const lines = createInterface({ input: child.stderr })
    lines.on('line', (line) => {
      log.write(`${line}\n`)
      const fields = logFields(line)
      if (fields === undefined) return
      if (fields.message === 'listening' && fields.listener === 'client_api') {
        addr = String(fields.addr)
      }
      if (typeof fields.link === 'string') setupPath = fields.link
      if (fields.message !== 'ready') return
      clearTimeout(timer)
      child.removeAllListeners('exit')
      if (addr === undefined || setupPath === undefined) {
        fail(
          'ready without a client API address or a first-run setup link (is the database empty?)',
        )
        return
      }
      resolve({ baseUrl: `http://${addr}`, setupPath, stop })
    })
  })
}
