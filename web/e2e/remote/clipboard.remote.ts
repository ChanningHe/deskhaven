import { randomBytes } from 'node:crypto'

import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import type { ClientKey } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { clipboardOf, setClipboard } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']
const QUIET_MS = 3_000

const writeLocal = (page: Page, text: string) =>
  page.evaluate((value) => navigator.clipboard.writeText(value), text)
const readLocal = (page: Page) => page.evaluate(() => navigator.clipboard.readText())

for (const key of VERSIONS) {
  test(`clipboard: text goes both ways with a ${key} device, without clicks`, async ({
    page,
    context,
  }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const tag = randomBytes(6).toString('hex')
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])

    await openFromDeviceList(page, target.id)
    const before = `before-${tag}`
    await writeLocal(page, before)
    await submitPassword(page)
    await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')
    await page.waitForTimeout(QUIET_MS)
    expect(clipboardOf(target), 'what was copied before the session').not.toBe(before)

    const here = `here-${tag}`
    await writeLocal(page, here)
    await expect.poll(() => clipboardOf(target), { message: 'the device clipboard' }).toBe(here)
    // The device's clipboard listener can miss a change that closely follows its own write.
    await page.waitForTimeout(QUIET_MS)

    const there = `there-${tag}`
    setClipboard(target, there)
    await expect.poll(() => readLocal(page), { message: 'the local clipboard' }).toBe(there)
    await page.waitForTimeout(QUIET_MS)
    expect(clipboardOf(target), 'the device text is not sent back over it').toBe(there)
  })
}
