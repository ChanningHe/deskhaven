// Display > Codec mid-session: the 1.5 client re-encodes in the picked codec, the stats overlay
// names it, the picture keeps moving, and VP9 comes back the same way.
import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'
import { xclock } from './support/x11.ts'

const VP9 = 'vp09.00.10.08'
const CODECS = [
  { name: 'VP8', codec: 'vp8' },
  { name: 'AV1', codec: 'av01.0.04M.08' },
] as const

/** A digest of the canvas pixels: changes when a new picture is drawn. */
const canvasDigest = (page: Page) =>
  page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('canvas[aria-hidden]')
    const context = canvas?.getContext('2d')
    if (!canvas || !context || canvas.width === 0) return 0
    const { data } = context.getImageData(0, 0, canvas.width, canvas.height)
    let hash = 0
    for (let i = 0; i < data.length; i += 97) hash = (hash * 31 + (data[i] ?? 0)) | 0
    return hash
  })

async function pickCodec(page: Page, name: string) {
  await page.getByRole('button', { name: 'Display' }).click()
  await page.getByRole('menuitem', { name: /^Codec/ }).focus()
  await page.keyboard.press('ArrowRight')
  await page.getByRole('menuitemradio', { name: new RegExp(`^${name}`) }).click()
}

async function expectStream(page: Page, codec: string) {
  const stat = (name: string) => page.locator(`[data-stat="${name}"]`)
  await expect(stat('codec')).toHaveAttribute('data-value', codec, { timeout: 30_000 })
  await expect
    .poll(async () => Number(await stat('fps').getAttribute('data-value')))
    .toBeGreaterThan(0)
  const before = await canvasDigest(page)
  await expect.poll(() => canvasDigest(page), { timeout: 10_000 }).not.toBe(before)
  await expect(page.getByText(/switched back to VP9/)).toHaveCount(0)
}

for (const { name, codec } of CODECS) {
  test(`codec: a 1.5 client switches to ${name} mid-session and back to VP9`, async ({ page }) => {
    const target = device('1.5')
    test.skip(target === undefined, 'no 1.5 client in this run')
    if (target === undefined) return

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, 'WebRTC')
    const clock = xclock(target)
    try {
      await page.getByRole('button', { name: 'Display' }).click()
      await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
      await expectStream(page, VP9)

      await pickCodec(page, name)
      await expectStream(page, codec)

      await pickCodec(page, 'VP9')
      await expectStream(page, VP9)
    } finally {
      clock.stop()
    }
  })
}
