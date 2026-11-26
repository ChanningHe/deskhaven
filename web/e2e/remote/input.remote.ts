// Only unshifted keys: Playwright's `keyboard.type` sets no Shift for them, and the modes under
// test must agree on exactly these keys. Ctrl+D ends the line so `tee` flushes.
import { randomBytes } from 'node:crypto'
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
import { focus, xterm } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']
const MODES = ['Translate', 'Map'] as const
/** Inside the xterm (484x316 at the top left of the 1280x800 screen). */
const CLICK_AT = { x: 0.15, y: 0.15 }
/** The black xterm is 15% of the white screen; the client's own windows are far smaller. */
const XTERM_SHARE = 0.1

async function clickAt(screen: Locator) {
  const box = await screen.boundingBox()
  if (box === null) throw new Error('the remote screen has no box')
  await screen.click({ position: { x: box.width * CLICK_AT.x, y: box.height * CLICK_AT.y } })
}

for (const key of VERSIONS) {
  for (const mode of MODES) {
    test(`input: ${mode} keyboard mode types into a ${key} device`, async ({ page }) => {
      const target = device(key)
      test.skip(target === undefined, `no ${key} client in this run`)
      if (target === undefined) return
      // Lower case: the typed keys carry no Shift, and the name is typed too.
      const name = `typed-${key}-${mode}`.toLowerCase()
      const file = join(target.scratchDir, name)
      await rm(file, { force: true })
      const text = `e2e${randomBytes(6).toString('hex')} ${randomBytes(3).toString('hex')}`

      await openFromDeviceList(page, target.id)
      await submitPassword(page)
      await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')
      await page.getByRole('button', { name: 'Input' }).click()
      await page.getByRole('menuitemradio', { name: mode }).click()

      const term = xterm(target)
      try {
        await expect
          .poll(() => frameShare(page, 'dark'), { timeout: 30_000 })
          .toBeGreaterThan(XTERM_SHARE)
        focus(target, 'xterm')
        await clickAt(page.getByTestId('remote-screen'))
        await page.keyboard.type(`tee ${name}`)
        await page.keyboard.press('Enter')
        await page.keyboard.type(text)
        await page.keyboard.press('Enter')
        await page.keyboard.press('Control+d')

        await expect
          .poll(() => readFile(file, 'utf8').catch(() => ''), { message: 'what the device typed' })
          .toBe(`${text}\n`)
      } finally {
        term.stop()
      }
    })
  }
}
