// `test` for every spec: the base URL is the server the global setup started (its port
// is only known once it listens, after the config was read).
import { test as base } from '@playwright/test'

import { BASE_URL_ENV, required } from './env.ts'

interface Server {
  /** The server's origin, for calls of the client API. */
  serverUrl: string
}

export const test = base.extend<Server>({
  // eslint-disable-next-line no-empty-pattern -- Playwright fixtures destructure their deps
  serverUrl: async ({}, provide) => {
    await provide(required(BASE_URL_ENV))
  },
  baseURL: async ({ serverUrl }, provide) => {
    await provide(serverUrl)
  },
})

export { expect } from '@playwright/test'
