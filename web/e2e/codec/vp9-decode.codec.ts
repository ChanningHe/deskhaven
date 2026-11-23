// The remote client's video pipeline on real WebCodecs: the captured VP9 key frame of the
// relay fixture decodes to the peer's 1280×800 screen.
import { fileURLToPath } from 'node:url'

import { expect, test } from '@playwright/test'
import { build } from 'vite'

const WEB_ROOT = fileURLToPath(new URL('../..', import.meta.url))
const ENTRY = fileURLToPath(
  new URL('../../src/features/remote/video/decode-check.ts', import.meta.url),
)
const GLOBAL = 'decodeCheck'
// VideoDecoder exists only in secure contexts; an https page is one.
const ORIGIN = 'https://codec.test'

interface DecodeCheckResult {
  supported: boolean
  codedWidth: number
  displayWidth: number
  displayHeight: number
  visibleWidth: number
  keyRequests: number
}

async function bundle(): Promise<string> {
  const result = await build({
    configFile: false,
    root: WEB_ROOT,
    logLevel: 'silent',
    build: {
      write: false,
      minify: false,
      lib: { entry: ENTRY, formats: ['iife'], name: GLOBAL, fileName: 'check' },
    },
  })
  const outputs = Array.isArray(result) ? result : [result]
  for (const out of outputs) {
    if (!('output' in out)) continue
    const chunk = out.output.find((o) => o.type === 'chunk')
    if (chunk?.type === 'chunk') return chunk.code
  }
  throw new Error('vite produced no script chunk')
}

test('decodes the captured VP9 key frame through the pipeline', async ({ page }) => {
  const script = await bundle()
  await page.route(`${ORIGIN}/check.js`, (route) =>
    route.fulfill({ contentType: 'text/javascript', body: script }),
  )
  await page.route(`${ORIGIN}/`, (route) =>
    route.fulfill({
      contentType: 'text/html',
      body: '<!doctype html><title>decode check</title><script src="/check.js"></script>',
    }),
  )
  await page.goto(`${ORIGIN}/`)

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
