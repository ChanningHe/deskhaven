import { readFile, rm } from 'node:fs/promises'
import { join } from 'node:path'

import type { Locator } from '@playwright/test'

import {
  device,
  expectDesktop,
  frameShare,
  openFromDeviceList,
  submitPassword,
} from './support/console.ts'
import type { ClientKey } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { focus, keyProbe, movePointer, pointerAt } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']
const CLICK_AT = { x: 0.15, y: 0.15 }
const XTERM_SHARE = 0.1
const PARKED = { x: 900, y: 600 }
const SETTLE_MS = 2000

async function clickAt(screen: Locator) {
  const box = await screen.boundingBox()
  if (box === null) throw new Error('the remote screen has no box')
  await screen.click({ position: { x: box.width * CLICK_AT.x, y: box.height * CLICK_AT.y } })
}

for (const key of VERSIONS) {
  test(`view only: nothing reaches a ${key} device until it is off`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const name = `view-only-${key}`
    const file = join(target.scratchDir, name)
    await rm(file, { force: true })

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')

    const probe = keyProbe(target, name)
    try {
      await expect
        .poll(() => frameShare(page, 'dark'), { timeout: 30_000 })
        .toBeGreaterThan(XTERM_SHARE)
      focus(target, 'xterm')
      await page.getByRole('button', { name: 'Actions' }).click()
      await page.getByRole('menuitemcheckbox', { name: 'View only' }).click()
      await expect(page.getByRole('banner').getByText('View only')).toBeVisible()

      movePointer(target, PARKED.x, PARKED.y)
      const screen = page.getByTestId('remote-screen')
      await clickAt(screen)
      await page.keyboard.type('blocked line')
      await page.keyboard.press('Enter')
      await page.waitForTimeout(SETTLE_MS)
      expect(pointerAt(target)).toEqual(PARKED)
      expect(await readFile(file, 'utf8')).toBe('')

      await page.getByRole('button', { name: 'Actions' }).click()
      await page.getByRole('menuitemcheckbox', { name: 'View only' }).click()
      await expect(page.getByRole('banner').getByText('View only')).toBeHidden()
      await clickAt(screen)
      await page.keyboard.type('live line')
      await page.keyboard.press('Enter')
      await expect
        .poll(() => readFile(file, 'utf8').catch(() => ''), { message: 'what the device typed' })
        .toBe('live line\n')
      expect(pointerAt(target)).not.toEqual(PARKED)
    } finally {
      probe.stop()
    }
  })
}
