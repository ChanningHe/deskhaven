import type { Page } from '@playwright/test'
import { expect } from '@playwright/test'

import { DEVICE_PASSWORD, OWNER, stackInfo } from './env.ts'
import type { ClientKey, DeviceInfo } from './env.ts'

export function device(key: ClientKey): DeviceInfo | undefined {
  return stackInfo().devices[key]
}

export async function signIn(page: Page) {
  await page.goto('/login')
  await page.getByLabel('Username').fill(OWNER.username)
  await page.getByLabel('Password').fill(OWNER.password)
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible()
}

export async function openFromDeviceList(page: Page, id: string) {
  await page.goto('/devices')
  await page.getByText(id, { exact: true }).first().click()
  await page.getByRole('link', { name: 'Connect in browser' }).click()
  await expect(page).toHaveURL(new RegExp(`/remote/${id}$`))
}

// A link opened from outside the console asks before it connects.
export async function openFromLink(page: Page, id: string) {
  await page.goto(`/remote/${id}`)
  await page.getByRole('button', { name: 'Connect', exact: true }).click()
}

export async function waitOnline(page: Page, id: string) {
  await expect
    .poll(
      async () => {
        // From inside the page: the session cookie is the browser's own.
        return page.evaluate(async (deviceId) => {
          const reply = await fetch(`/admin/api/v1/devices?q=${deviceId}`)
          if (!reply.ok) return `HTTP ${String(reply.status)}`
          const body = (await reply.json()) as {
            items?: { rustdesk_id: string; online: boolean }[]
          }
          const found = body.items?.find((d) => d.rustdesk_id === deviceId)
          return found === undefined ? 'absent' : found.online ? 'online' : 'offline'
        }, id)
      },
      { timeout: 60_000, message: `device ${id} online in the console` },
    )
    .toBe('online')
}

export async function submitPassword(page: Page) {
  await page.getByRole('textbox', { name: 'Password' }).fill(DEVICE_PASSWORD)
  await page.getByRole('button', { name: 'Connect' }).click()
}

/** 0 while no frame has been drawn. */
export async function frameShare(page: Page, kind: 'lit' | 'dark' = 'lit'): Promise<number> {
  return page.evaluate((wanted) => {
    const canvas = document.querySelector<HTMLCanvasElement>('canvas[aria-hidden]')
    if (canvas === null || canvas.width === 0) return 0
    const context = canvas.getContext('2d')
    if (context === null) return 0
    const { data } = context.getImageData(0, 0, canvas.width, canvas.height)
    let hits = 0
    for (let i = 0; i < data.length; i += 4) {
      if ((data[i + 3] ?? 0) === 0) continue
      const bright = (data[i] ?? 0) | (data[i + 1] ?? 0) | (data[i + 2] ?? 0)
      if (bright > 8 === (wanted === 'lit')) hits += 1
    }
    return hits / (data.length / 4)
  }, kind)
}

export async function expectDesktop(page: Page, transport: 'Relay' | 'WebRTC') {
  const pill = page.locator('header span.rounded-full', { hasText: 'Connected' })
  await expect(pill).toContainText(transport)
  await expect.poll(() => frameShare(page), { timeout: 30_000 }).toBeGreaterThan(0.5)
}
