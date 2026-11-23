import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import type { ClientKey } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { addMode, setMode } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']
const SCREEN = { width: 1280, height: 800 }
const SMALL = { width: 1024, height: 768 }

const canvasSize = (page: Page) =>
  page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('canvas[aria-hidden]')
    return canvas === null ? null : `${String(canvas.width)}x${String(canvas.height)}`
  })

async function openResolutions(page: Page) {
  await page.getByRole('button', { name: 'Display' }).click()
  const entry = page.getByRole('menuitem', { name: /^Resolution/ })
  await entry.focus()
  await page.keyboard.press('ArrowRight')
}

for (const key of VERSIONS) {
  test(`resolution: ${key} device switches between its sizes`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const small = `${String(SMALL.width)}x${String(SMALL.height)}`
    const full = `${String(SCREEN.width)}x${String(SCREEN.height)}`

    try {
      addMode(target, SMALL.width, SMALL.height)
      await openFromDeviceList(page, target.id)
      await submitPassword(page)
      await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')
      await expect.poll(() => canvasSize(page)).toBe(full)

      await openResolutions(page)
      await expect(page.getByRole('menuitemradio', { name: 'Original (1280×800)' })).toBeChecked()
      await page.getByRole('menuitemradio', { name: '1024×768' }).click()
      await expect.poll(() => canvasSize(page), { timeout: 30_000 }).toBe(small)

      await openResolutions(page)
      await expect(page.getByRole('menuitemradio', { name: '1024×768' })).toBeChecked()
      await page.getByRole('menuitemradio', { name: 'Original (1280×800)' }).click()
      await expect.poll(() => canvasSize(page), { timeout: 30_000 }).toBe(full)
    } finally {
      setMode(target, SCREEN.width, SCREEN.height)
    }
  })
}
