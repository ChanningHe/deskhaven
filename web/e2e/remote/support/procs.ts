// Child processes of the stack: each in its own process group (the client and Xvfb start
// helpers of their own), output to a log file, the whole group stopped at the end.
import { spawn } from 'node:child_process'
import { closeSync, openSync } from 'node:fs'

const STOP_GRACE_MS = 5_000

export interface Managed {
  name: string
  stop: () => Promise<void>
}

export function spawnManaged(
  name: string,
  command: string,
  args: string[],
  options: { env: NodeJS.ProcessEnv; logFile: string; cwd?: string },
): Managed {
  const fd = openSync(options.logFile, 'a')
  const child = spawn(command, args, {
    env: options.env,
    cwd: options.cwd,
    stdio: ['ignore', fd, fd],
    detached: true,
  })
  closeSync(fd)
  let gone = child.exitCode !== null
  const exited = new Promise<void>((resolve) => {
    child.once('exit', () => {
      gone = true
      resolve()
    })
  })
  child.once('error', (error) => {
    process.stderr.write(`${name}: ${error.message}\n`)
    gone = true
  })
  const signal = (sig: NodeJS.Signals) => {
    if (child.pid === undefined) return
    try {
      process.kill(-child.pid, sig)
    } catch {
      // The group is already gone.
    }
  }
  return {
    name,
    stop: async () => {
      if (!gone) signal('SIGTERM')
      const timer = setTimeout(() => {
        signal('SIGKILL')
      }, STOP_GRACE_MS)
      if (!gone) await exited
      clearTimeout(timer)
      signal('SIGKILL')
    },
  }
}

/** Resolves when `ready` returns true; polls every 250 ms. */
export async function waitFor(what: string, ready: () => boolean | Promise<boolean>, ms: number) {
  const deadline = Date.now() + ms
  for (;;) {
    if (await ready()) return
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`)
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
}
