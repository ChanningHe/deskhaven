// Lock screen and Ctrl+Alt+Del leave no trace on a bare Xvfb.
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
import { focus, keyProbe } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']
const KEYS = [
  { item: 'Alt+Tab', line: 'alt-tab' },
  { item: 'PrintScreen', line: 'print-screen' },
  { item: 'Win key', line: 'win' },
]
const CLICK_AT = { x: 0.15, y: 0.15 }
const XTERM_SHARE = 0.1

async function clickAt(screen: Locator) {
  const box = await screen.boundingBox()
  if (box === null) throw new Error('the remote screen has no box')
  await screen.click({ position: { x: box.width * CLICK_AT.x, y: box.height * CLICK_AT.y } })
}

for (const key of VERSIONS) {
  test(`send keys: Alt+Tab, PrintScreen and Win reach a ${key} device`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const name = `sent-keys-${key}`
    await rm(join(target.scratchDir, name), { force: true })

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')

    const probe = keyProbe(target, name)
    try {
      await expect
        .poll(() => frameShare(page, 'dark'), { timeout: 30_000 })
        .toBeGreaterThan(XTERM_SHARE)
      focus(target, 'xterm')
      await clickAt(page.getByTestId('remote-screen'))
      for (const { item } of KEYS) {
        await page.getByRole('button', { name: 'Input' }).click()
        await page.getByRole('menuitem', { name: 'Send keys' }).click()
        await page.getByRole('menuitem', { name: item }).click()
      }
      await expect
        .poll(() => readFile(join(target.scratchDir, name), 'utf8').catch(() => ''), {
          message: 'what the device saw',
        })
        .toBe(KEYS.map(({ line }) => `${line}\n`).join(''))
    } finally {
      probe.stop()
    }
  })
}
