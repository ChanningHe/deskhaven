// `test` for every remote spec: the base URL is the proxy the global setup started, and a
// test's page ends its session with "Disconnect". Closing the browser context does not fire
// `pagehide`, so without it each test would hold one of the user's 4 session slots until
// the server's session timeout, and the fifth test in a minute would be refused.
import { test as base } from '@playwright/test'

import { BASE_URL_ENV } from './env.ts'

const DISCONNECT_WAIT_MS = 2_000

export const test = base.extend({
  // eslint-disable-next-line no-empty-pattern -- Playwright fixtures destructure their deps
  baseURL: async ({}, provide) => {
    const url = process.env[BASE_URL_ENV]
    if (url === undefined || url === '')
      throw new Error(`${BASE_URL_ENV} is not set (global setup)`)
    await provide(url)
  },
  page: async ({ page }, provide) => {
    await provide(page)
    const disconnect = page.getByRole('button', { name: 'Disconnect' })
    if (await disconnect.isVisible().catch(() => false)) {
      await disconnect.click({ timeout: DISCONNECT_WAIT_MS }).catch(() => undefined)
      await page.waitForTimeout(DISCONNECT_WAIT_MS / 4)
    }
  },
})

export { expect } from '@playwright/test'
