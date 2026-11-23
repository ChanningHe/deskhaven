import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import type { ClientKey } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { movePointer } from './support/x11.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']

const overlayPainted = (page: Page) =>
  page.getByTestId('remote-screen').evaluate((element) => {
    if (!(element instanceof HTMLCanvasElement)) return 0
    const context = element.getContext('2d')
    if (context === null) return 0
    const { data } = context.getImageData(0, 0, element.width, element.height)
    let painted = 0
    for (let i = 3; i < data.length; i += 4) if ((data[i] ?? 0) > 0) painted += 1
    return painted
  })

for (const key of VERSIONS) {
  test(`cursor: a ${key} device's cursor is the local pointer`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    const violations: string[] = []
    page.on('console', (message) => {
      if (message.text().includes('Content Security Policy')) violations.push(message.text())
    })

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, key === '1.4' ? 'Relay' : 'WebRTC')

    const screen = page.getByTestId('remote-screen')
    const box = await screen.boundingBox()
    if (box === null) throw new Error('the remote screen has no box')
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.move(box.x + box.width / 3, box.y + box.height / 3)
    await expect
      .poll(() => screen.evaluate((element) => getComputedStyle(element).cursor), {
        message: "the screen's CSS cursor",
      })
      .toMatch(/^url\("blob:/)
    expect(await overlayPainted(page)).toBe(0)

    let step = 0
    await expect
      .poll(
        () => {
          movePointer(target, 900 + (step++ % 2) * 20, 600)
          return overlayPainted(page)
        },
        { message: 'the peer cursor drawn on the screen' },
      )
      .toBeGreaterThan(0)
    expect(violations).toEqual([])
  })
}
