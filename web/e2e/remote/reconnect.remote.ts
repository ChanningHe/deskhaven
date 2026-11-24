import type { Page } from '@playwright/test'

import { device, expectDesktop, submitPassword } from './support/console.ts'
import { stackInfo } from './support/env.ts'
import type { ClientKey, DeviceInfo } from './support/env.ts'
import { test as base, expect } from './support/fixtures.ts'
import { deviceLog } from './support/peer-log.ts'
import { startProxy } from './support/proxy.ts'
import type { Proxy } from './support/proxy.ts'

// The peer keeps a lost session for 30 s (connection.rs:510).
const PEER_WINDOW_MS = 30_000
const GIVE_UP_WITHIN_MS = 60_000
// ~9 sessions here vs 10/min per user (limits.rs): later specs would be rate_limited.
const SESSION_BUDGET_REFILL_MS = 60_000
// The client logs no rejected login, only connections and accepted logins (connection.rs).
const OPENED = 'Connection opened from'
const LOGGED_IN = 'peer info supported_encoding'

const test = base.extend<{ relay: Proxy }>({
  // eslint-disable-next-line no-empty-pattern -- Playwright fixtures destructure their deps
  relay: async ({}, provide) => {
    const url = new URL(stackInfo().baseUrl)
    const stack = { host: url.hostname, port: Number(url.port) }
    const proxy = await startProxy(stack, stack)
    await provide(proxy)
    await proxy.stop()
  },
  baseURL: async ({ relay }, provide) => {
    await provide(relay.origin)
  },
})

function target(): { key: ClientKey; info: DeviceInfo } | undefined {
  for (const key of ['1.5', '1.4'] as const) {
    const info = device(key)
    if (info) return { key, info }
  }
  return undefined
}

async function prepare(page: Page) {
  await page.addInitScript(() => {
    Reflect.deleteProperty(window, 'RTCPeerConnection')
    const counted = window as unknown as { passwordPrompts: number }
    counted.passwordPrompts = 0
    let shown = false
    new MutationObserver(() => {
      const now = document.querySelector('input[type="password"]') !== null
      if (now && !shown) counted.passwordPrompts += 1
      shown = now
    }).observe(document, { childList: true, subtree: true })
  })
}

function passwordPrompts(page: Page): Promise<number> {
  return page.evaluate(() => (window as unknown as { passwordPrompts: number }).passwordPrompts)
}

function deviceLogins(key: ClientKey, info: DeviceInfo): { opened: number; in: number } | null {
  const log = deviceLog(key, info)
  if (log === null) return null
  return { opened: log.split(OPENED).length - 1, in: log.split(LOGGED_IN).length - 1 }
}

async function connect(page: Page, info: DeviceInfo) {
  await page.goto(`/remote/${info.id}`)
  await submitPassword(page)
  await expectDesktop(page, 'Relay')
  await page.evaluate(() => {
    ;(window as unknown as { passwordPrompts: number }).passwordPrompts = 0
  })
}

test('a short relay cut reconnects without the password, and the device accepts it', async ({
  page,
  relay,
}) => {
  const found = target()
  test.skip(found === undefined, 'no client in this run')
  if (found === undefined) return
  await prepare(page)
  await connect(page, found.info)
  const before = deviceLogins(found.key, found.info)

  const cut = Date.now()
  relay.dropRelay()
  await expect(page.getByText(/Reconnecting \(attempt 1\)/)).toBeVisible()
  await expectDesktop(page, 'Relay')
  expect(Date.now() - cut).toBeLessThan(PEER_WINDOW_MS)
  expect(await passwordPrompts(page)).toBe(0)

  const after = deviceLogins(found.key, found.info)
  test.info().annotations.push({
    type: 'device log',
    description: `before the cut ${JSON.stringify(before)}, after ${JSON.stringify(after)}`,
  })
  if (before === null || after === null) return
  expect(after).toEqual({ opened: before.opened + 1, in: before.in + 1 })
})

test('a cut past the reconnect budget asks for the password once, on reconnect', async ({
  page,
  relay,
}) => {
  test.setTimeout(GIVE_UP_WITHIN_MS + 90_000)
  const found = target()
  test.skip(found === undefined, 'no client in this run')
  if (found === undefined) return
  await prepare(page)
  await connect(page, found.info)

  relay.refuseRelay(true)
  relay.dropRelay()
  await expect(page.getByText(/Reconnecting \(attempt 1\)/)).toBeVisible()
  await expect(page.getByText(/The connection to the relay was lost/)).toBeVisible({
    timeout: GIVE_UP_WITHIN_MS,
  })
  expect(await passwordPrompts(page)).toBe(0)

  relay.refuseRelay(false)
  await page.getByRole('button', { name: 'Reconnect' }).click()
  await expect(page.getByRole('textbox', { name: 'Password' })).toBeVisible()
  await submitPassword(page)
  await expectDesktop(page, 'Relay')
  expect(await passwordPrompts(page)).toBe(1)
})

test.afterAll(async () => {
  test.setTimeout(SESSION_BUDGET_REFILL_MS + 10_000)
  await new Promise((resolve) => setTimeout(resolve, SESSION_BUDGET_REFILL_MS))
})
