// The deskhaven binary under test: a fresh data dir, loopback listeners, JSON logs on
// stderr. Startup is over when the log says "ready"; the first-run setup link and the
// client API address are read from the same log.
import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { once } from 'node:events'
import { createWriteStream } from 'node:fs'
import { createInterface } from 'node:readline'
import { Readable } from 'node:stream'

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

/** The child's stderr copied to `logFile`; resolves once the copy is complete on disk. */
function copyLog(child: ChildProcess, logFile: string, onLine: (line: string) => void) {
  const log = createWriteStream(logFile)
  const lines = createInterface({ input: child.stderr ?? Readable.from([]) })
  lines.on('line', (line) => {
    log.write(`${line}\n`)
    onLine(line)
  })
  return (async () => {
    await once(lines, 'close')
    log.end()
    await once(log, 'finish')
  })()
}

/** A process that has closed its stdio (`close`, after `exit`) and whose log is flushed. */
async function settle(closed: Promise<unknown>, flushed: Promise<void>) {
  await closed
  await flushed
}

/** SIGTERM, then SIGKILL after the grace period; resolves once the log is complete. */
function stopper(child: ChildProcess, closed: Promise<unknown>, flushed: Promise<void>) {
  return async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
    let timer: NodeJS.Timeout | undefined
    const timedOut = new Promise<boolean>((resolve) => {
      timer = setTimeout(() => {
        resolve(true)
      }, SHUTDOWN_TIMEOUT_MS)
    })
    const late = await Promise.race([closed.then(() => false), timedOut])
    clearTimeout(timer)
    if (late) child.kill('SIGKILL')
    await settle(closed, flushed)
    if (late) throw new Error(`deskhaven did not stop within ${SHUTDOWN_TIMEOUT_MS} ms`)
  }
}

/** Starts `deskhaven serve` and resolves once it is ready, with what the tests need. */
export function startServer(options: ServerOptions): Promise<RunningServer> {
  const child = spawn(options.binary, ['serve'], {
    cwd: options.dataDir,
    env: serverEnv(options),
    stdio: ['ignore', 'ignore', 'pipe'],
  })
  const closed = new Promise<void>((resolve) => {
    child.once('close', () => {
      resolve()
    })
  })
  let addr: string | undefined
  let setupPath: string | undefined
  let ready: (() => void) | undefined
  const flushed = copyLog(child, options.logFile, (line) => {
    const fields = logFields(line)
    if (fields === undefined) return
    if (fields.message === 'listening' && fields.listener === 'client_api') {
      addr = String(fields.addr)
    }
    if (typeof fields.link === 'string') setupPath = fields.link
    if (fields.message === 'ready') ready?.()
  })

  return new Promise((resolve, reject) => {
    let failed = false
    /** Kills the child and rejects once its log is complete (`flush`: it ever ran). */
    const fail = (reason: string, flush = true) => {
      if (failed) return
      failed = true
      clearTimeout(timer)
      child.kill('SIGKILL')
      const error = new Error(`${reason}; server log: ${options.logFile}`)
      if (!flush) {
        reject(error)
        return
      }
      void settle(closed, flushed).finally(() => {
        reject(error)
      })
    }
    child.once('error', (error) => {
      fail(`cannot start ${options.binary}: ${error.message}`, false)
    })
    const timer = setTimeout(() => {
      fail(`deskhaven was not ready within ${STARTUP_TIMEOUT_MS} ms`)
    }, STARTUP_TIMEOUT_MS)
    const onEarlyExit = (code: number | null, signal: string | null) => {
      fail(`deskhaven exited during startup (code ${String(code)}, signal ${String(signal)})`)
    }
    child.once('exit', onEarlyExit)
    ready = () => {
      if (failed) return
      clearTimeout(timer)
      child.off('exit', onEarlyExit)
      if (addr === undefined || setupPath === undefined) {
        fail('ready without a client API address or a first-run setup link (empty database?)')
        return
      }
      let stopping = false
      child.once('exit', (code, signal) => {
        if (stopping) return
        process.stderr.write(
          `deskhaven exited unexpectedly (code ${String(code)}, signal ${String(signal)}); ` +
            `server log: ${options.logFile}\n`,
        )
      })
      const stop = stopper(child, closed, flushed)
      resolve({
        baseUrl: `http://${addr}`,
        setupPath,
        stop: () => {
          stopping = true
          return stop()
        },
      })
    }
  })
}
