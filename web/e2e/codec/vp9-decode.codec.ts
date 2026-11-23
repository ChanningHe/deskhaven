// The remote client's video pipeline on real WebCodecs: the captured VP9 key frame of the
// relay fixture decodes to the peer's 1280×800 screen.
import { expect, test } from '@playwright/test'

import { openCheck } from './bundle.ts'

const GLOBAL = 'decodeCheck'

interface DecodeCheckResult {
  supported: boolean
  codedWidth: number
  displayWidth: number
  displayHeight: number
  visibleWidth: number
  keyRequests: number
}

test('decodes the captured VP9 key frame through the pipeline', async ({ page }) => {
  await openCheck(page, 'src/features/remote/video/decode-check.ts', GLOBAL)

  const result = await page.evaluate(async (name) => {
    const check = (globalThis as unknown as Record<string, unknown>)[name] as {
      decodeCapturedKeyFrame: () => Promise<DecodeCheckResult>
    }
    return check.decodeCapturedKeyFrame()
  }, GLOBAL)

  expect(result).toMatchObject({
    supported: true,
    displayWidth: 1280,
    displayHeight: 800,
    visibleWidth: 1280,
    keyRequests: 0,
  })
  expect(result.codedWidth).toBeGreaterThanOrEqual(1280)
})
