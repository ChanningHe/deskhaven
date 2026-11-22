// (f) @slow: a long session with a screen that keeps changing. Run on request only
// (RD_E2E_SLOW=1). Reports the time to the first frame and the end-to-end latency of a
// change on the device (a black window mapped on a white screen, seen in the video canvas)
// at the start and then once a minute; the latency must not grow.
import { execFileSync } from 'node:child_process'

import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { tool } from './support/env.ts'
import type { ClientKey, DeviceInfo } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { xclock, xterm } from './support/x11.ts'

const SOAK_MS = Number(process.env.RD_E2E_SOAK_SECONDS ?? '300') * 1000
const SAMPLE_EVERY_MS = 60_000
/**
 * The probe window (an xterm) sits away from the top left, where the client's own windows
 * and the clock are; PROBE is a pixel inside it, as a share of the screen.
 */
const PROBE_AT = '+700+100'
const PROBE = { x: 0.62, y: 0.25 }
/** The latency at the end may be this many times the start's, plus the slack. */
const GROWTH = 3
const SLACK_MS = 400

function xdo(target: DeviceInfo, ...args: string[]) {
  execFileSync(tool('xdotool'), args, {
    env: { PATH: process.env.PATH ?? '', DISPLAY: target.display },
    timeout: 10_000,
  })
}

/** Milliseconds (device clock = this clock) until the probe pixel of the canvas is `dark`. */
function pixelIs(page: Page, dark: boolean, timeoutMs: number): Promise<number> {
  return page.evaluate(
    ({ probe, wantDark, limit }) =>
      new Promise<number>((resolve, reject) => {
        const canvas = document.querySelector<HTMLCanvasElement>('canvas[aria-hidden]')
        const context = canvas?.getContext('2d')
        if (!canvas || !context) {
          reject(new Error('no video canvas'))
          return
        }
        const started = Date.now()
        const look = () => {
          const px = context.getImageData(
            Math.floor(canvas.width * probe.x),
            Math.floor(canvas.height * probe.y),
            1,
            1,
          ).data
          const isDark = (px[0] ?? 255) < 64 && (px[3] ?? 0) > 0
          if (isDark === wantDark) resolve(Date.now())
          else if (Date.now() - started > limit) reject(new Error('probe pixel never changed'))
          else requestAnimationFrame(look)
        }
        look()
      }),
    { probe: PROBE, wantDark: dark, limit: timeoutMs },
  )
}

/** Map the probe window, return how long the canvas took to show it; then unmap it again. */
async function latency(page: Page, target: DeviceInfo): Promise<number> {
  xdo(target, 'search', '--class', 'xterm', 'windowunmap')
  await pixelIs(page, false, 20_000)
  const seen = pixelIs(page, true, 20_000)
  const sent = Date.now()
  xdo(target, 'search', '--class', 'xterm', 'windowmap')
  return (await seen) - sent
}

const CASES: [ClientKey, 'Relay' | 'WebRTC'][] = [
  ['1.5', 'WebRTC'],
  ['1.4', 'Relay'],
]

for (const [key, transport] of CASES) {
  test(`@slow soak: a ${key} session stays healthy over ${String(SOAK_MS / 1000)} s (${transport})`, async ({
    page,
  }) => {
    test.setTimeout(SOAK_MS + 120_000)
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return

    await openFromDeviceList(page, target.id)
    const clicked = Date.now()
    await submitPassword(page)
    await expectDesktop(page, transport)
    const firstFrameMs = Date.now() - clicked

    const probe = xterm(target, PROBE_AT)
    const clock = xclock(target)
    try {
      const samples: number[] = []
      const begin = Date.now()
      // The first sample waits for the probe window to be painted at all.
      await pixelIs(page, true, 30_000)
      const connected = page.locator('header span.rounded-full', { hasText: 'Connected' })
      samples.push(await latency(page, target))
      while (Date.now() - begin + SAMPLE_EVERY_MS < SOAK_MS) {
        await page.waitForTimeout(SAMPLE_EVERY_MS)
        // Still the same live session, on the same transport.
        await expect(connected).toContainText(transport)
        samples.push(await latency(page, target))
      }
      const wait = SOAK_MS - (Date.now() - begin)
      if (wait > 0) await page.waitForTimeout(wait)
      samples.push(await latency(page, target))

      const report = { key, transport, firstFrameMs, latencyMs: samples }
      process.stdout.write(`SOAK ${JSON.stringify(report)}\n`)
      await test
        .info()
        .attach('soak.json', { body: JSON.stringify(report), contentType: 'application/json' })
      const first = samples[0] ?? 0
      const last = samples[samples.length - 1] ?? 0
      expect(last).toBeLessThan(first * GROWTH + SLACK_MS)
    } finally {
      probe.stop()
      clock.stop()
    }
  })
}
