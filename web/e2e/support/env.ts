// What the global setup hands to the tests (through the environment, which Playwright
// passes on to its workers) and the fixed facts of the instance under test.
import { fileURLToPath } from 'node:url'

/** An empty PostgreSQL database to run on; unset means SQLite in the data dir. */
export const PG_URL_ENV = 'DESKHAVEN_E2E_PG_URL'
/** The binary to test; default: the workspace's debug build. */
export const BINARY_ENV = 'DESKHAVEN_E2E_BIN'

export const BASE_URL_ENV = 'E2E_BASE_URL'
export const SETUP_PATH_ENV = 'E2E_SETUP_PATH'
export const SIGNAL_PORT_ENV = 'E2E_SIGNAL_PORT'

export const DEFAULT_BINARY = fileURLToPath(
  new URL('../../../target/debug/deskhaven', import.meta.url),
)

/** Where reports, traces and the server log of this run go (one tree per backend). */
export function resultsDir(): string {
  const backend = process.env[PG_URL_ENV] ? 'postgres' : 'sqlite'
  return fileURLToPath(new URL(`../../e2e-results/${backend}`, import.meta.url))
}

/** The owner's session, saved by the first-run project for the others. */
export const OWNER_STATE = fileURLToPath(new URL('../.auth/owner.json', import.meta.url))

/**
 * Pinned by environment for the whole run: admins must enrol TOTP (the first-run flow
 * binds it), and the settings page must show the row read-only with this variable.
 */
export const PINNED = {
  key: 'auth.tfa.enforce',
  env: 'DESKHAVEN_SETTINGS__AUTH__TFA__ENFORCE',
  value: 'admins',
} as const

export const OWNER = {
  username: 'owner',
  password: 'e2e owner passphrase 1',
} as const

/** A variable the global setup must have set. */
export function required(name: string): string {
  const value = process.env[name]
  if (value === undefined || value === '') throw new Error(`${name} is not set (global setup)`)
  return value
}
