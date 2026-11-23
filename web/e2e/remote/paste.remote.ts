import { randomBytes } from 'node:crypto'
import { readFile, rm } from 'node:fs/promises'
import { join } from 'node:path'

import type { Page } from '@playwright/test'

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
const SHORTCUTS = ['Control+v', 'Control+Shift+V', 'Shift+Insert']
const CLICK_AT = { x: 0.15, y: 0.15 }
const XTERM_SHARE = 0.1
const SETTLE_MS = 800

async function copyLocally(page: Page, text: string) {
  await page.evaluate((value) => {
    const field = document.createElement('textarea')
    field.id = 'copy-source'
    field.value = value
    document.body.append(field)
    field.focus()
    field.select()
  }, text)
  await page.keyboard.press('Control+c')
  await page.evaluate(() => {
    document.getElementById('copy-source')?.remove()
  })
}

for (const key of VERSIONS) {
  test(`paste: shortcuts paste the local clipboard on a ${key} device`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const tag = randomBytes(6).toString('hex')
    const name = `pasted-${key}`.toLowerCase()
    const file = join(target.scratchDir, name)
    await rm(file, { force: true })

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')
    const term = xterm(target, '+0+0', true)
    try {
      await expect
        .poll(() => frameShare(page, 'dark'), { timeout: 30_000 })
        .toBeGreaterThan(XTERM_SHARE)
      await expect
        .poll(() => {
          try {
            focus(target, 'xterm')
            return true
          } catch {
            return false
          }
        })
        .toBe(true)
      const screen = page.getByTestId('remote-screen')
      const box = await screen.boundingBox()
      if (box === null) throw new Error('the remote screen has no box')
      const inside = { position: { x: box.width * CLICK_AT.x, y: box.height * CLICK_AT.y } }
      await screen.click(inside)
      await page.keyboard.type(`tee ${name}`)
      await page.keyboard.press('Enter')

      const texts = SHORTCUTS.map((_, i) => `pasted ${tag} ${String(i)}`)
      for (const [i, shortcut] of SHORTCUTS.entries()) {
        await copyLocally(page, texts[i] ?? '')
        await page.waitForTimeout(SETTLE_MS)
        await screen.click(inside)
        await page.keyboard.press(shortcut)
        await page.waitForTimeout(SETTLE_MS)
        await page.keyboard.press('Enter')
      }
      await page.keyboard.press('Control+d')

      await expect
        .poll(() => readFile(file, 'utf8').catch(() => ''), { message: 'what the device pasted' })
        .toBe(texts.map((t) => `${t}\n`).join(''))
    } finally {
      term.stop()
    }
  })
}
