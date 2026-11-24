// Display > Codec mid-session: the 1.5 client re-encodes in the picked codec, the stats overlay
// names it, the picture keeps moving, and VP9 comes back the same way. H264 and H265 need a
// hardware encoder the test peer lacks. The stats overlay saves raw frames as a fixture file.
import { readFile } from 'node:fs/promises'

import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'
import { flicker, xclock } from './support/x11.ts'

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

test('codec: H264 and H265 stay greyed out on a peer without hardware encoders', async ({
  page,
}) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'WebRTC')
  await page.getByRole('button', { name: 'Display' }).click()
  await page.getByRole('menuitem', { name: /^Codec/ }).focus()
  await page.keyboard.press('ArrowRight')
  for (const name of ['H264', 'H265']) {
    const item = page.getByRole('menuitemradio', { name: new RegExp(`^${name}`) })
    await expect(item).toHaveAttribute('aria-disabled', 'true')
    await expect(item).toHaveAccessibleName(`${name} (not supported by the device) Experimental`)
  }
})

test('sample: exports 30 raw VP9 frames from a key frame as JSON', async ({ page }) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'WebRTC')
  const motion = flicker(target, '+0+0', 40, 10)
  try {
    await page.getByRole('button', { name: 'Display' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
    await expectStream(page, VP9)

    const download = page.waitForEvent('download')
    await page.getByRole('button', { name: 'Export video sample' }).click()
    const file = await download
    expect(file.suggestedFilename()).toMatch(/^video-sample-vp9s-\d{8}-\d{6}\.json$/)
    const path = await file.path()
    const sample: unknown = JSON.parse(await readFile(path, 'utf8'))
    expect(sample).toMatchObject({
      version: 1,
      clientVersion: expect.any(String),
      codec: 'vp9s',
      complete: true,
      peer: { platform: 'Linux', version: expect.stringMatching(/^1\.5\./) },
      display: { width: expect.any(Number), height: expect.any(Number) },
    })
    const { frames, display } = sample as {
      frames: { key: boolean; pts: number; data: string }[]
      display: { width: number; height: number }
    }
    expect(display.width).toBeGreaterThan(0)
    expect(frames).toHaveLength(30)
    expect(frames[0]?.key).toBe(true)
    for (const frame of frames) {
      expect(typeof frame.pts).toBe('number')
      expect(Buffer.from(frame.data, 'base64').length).toBeGreaterThan(0)
    }
    await expect(page.getByRole('button', { name: 'Export video sample' })).toBeVisible()
  } finally {
    motion.stop()
  }
})
