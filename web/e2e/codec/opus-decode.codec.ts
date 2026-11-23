import { expect, test } from '@playwright/test'

import { openCheck } from './bundle.ts'

const GLOBAL = 'soundCheck'

interface SoundCheckResult {
  supported: boolean
  running: boolean
  packets: number
  largestPacket: number
  scheduled: number
  peak: number
  state: string
}

interface Format {
  sampleRate: number
  channels: number
}

const FORMATS: Format[] = [48000, 24000, 16000].flatMap((sampleRate) =>
  [1, 2].map((channels) => ({ sampleRate, channels })),
)

for (const format of FORMATS) {
  test(`plays a 10 ms-packet Opus tone at ${String(format.sampleRate)} Hz × ${String(format.channels)}`, async ({
    page,
  }) => {
    await openCheck(
      page,
      'src/features/remote/audio/decode-check.ts',
      GLOBAL,
      `<button id="play" onclick="window.result = ${GLOBAL}.playTone(window.format)">play</button>`,
    )
    await page.evaluate((told) => {
      ;(globalThis as unknown as { format: Format }).format = told
    }, format)
    // A real click: the browser lets an AudioContext run only after a gesture.
    await page.click('#play')
    const result = await page.evaluate(
      () => (globalThis as unknown as { result: Promise<SoundCheckResult> }).result,
    )

    expect(result).toMatchObject({ supported: true, running: true, state: 'ok' })
    expect(result.packets).toBeGreaterThanOrEqual(19)
    expect(result.largestPacket).toBeLessThanOrEqual(1276)
    expect(result.scheduled).toBe(result.packets)
    expect(result.peak).toBeGreaterThan(0.3)
  })
}
