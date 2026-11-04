// Starts one deskhaven for the whole run (fresh data dir; SQLite, or the empty
// PostgreSQL database of DESKHAVEN_E2E_PG_URL) and stops it in the returned teardown.
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  BASE_URL_ENV,
  BINARY_ENV,
  DEFAULT_BINARY,
  PG_URL_ENV,
  PINNED,
  SETUP_PATH_ENV,
  SIGNAL_PORT_ENV,
  resultsDir,
} from './support/env.ts'
import { freeSignalPort } from './support/ports.ts'
import { startServer } from './support/server.ts'

export default async function globalSetup(): Promise<() => Promise<void>> {
  const binary = process.env[BINARY_ENV] ?? DEFAULT_BINARY
  if (!existsSync(binary)) {
    throw new Error(`${binary} not found: build it (see e2e/README.md) or set ${BINARY_ENV}`)
  }
  const results = resultsDir()
  await mkdir(results, { recursive: true })
  const dataDir = await mkdtemp(join(tmpdir(), 'deskhaven-e2e-'))
  const pgUrl = process.env[PG_URL_ENV]
  const signalPort = await freeSignalPort()
  const server = await startServer({
    binary,
    dataDir,
    logFile: join(results, 'server.log'),
    signalPort,
    env: {
      [PINNED.env]: PINNED.value,
      ...(pgUrl ? { DESKHAVEN_DATABASE__URL: pgUrl } : {}),
    },
  })
  process.env[BASE_URL_ENV] = server.baseUrl
  process.env[SETUP_PATH_ENV] = server.setupPath
  process.env[SIGNAL_PORT_ENV] = String(signalPort)

  return async () => {
    await server.stop()
    await rm(dataDir, { recursive: true, force: true })
  }
}
