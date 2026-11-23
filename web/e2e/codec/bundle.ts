import { fileURLToPath } from 'node:url'

import type { Page } from '@playwright/test'
import { build } from 'vite'

const WEB_ROOT = fileURLToPath(new URL('../..', import.meta.url))
// WebCodecs exists only in secure contexts; an https page is one.
export const ORIGIN = 'https://codec.test'

async function bundle(entry: string, global: string): Promise<string> {
  const result = await build({
    configFile: false,
    root: WEB_ROOT,
    logLevel: 'silent',
    build: {
      write: false,
      minify: false,
      lib: {
        entry: fileURLToPath(new URL(`../../${entry}`, import.meta.url)),
        formats: ['iife'],
        name: global,
        fileName: 'check',
      },
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

export async function openCheck(
  page: Page,
  entry: string,
  global: string,
  body = '',
): Promise<void> {
  const script = await bundle(entry, global)
  await page.route(`${ORIGIN}/check.js`, (route) =>
    route.fulfill({ contentType: 'text/javascript', body: script }),
  )
  await page.route(`${ORIGIN}/`, (route) =>
    route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><title>codec check</title>${body}<script src="/check.js"></script>`,
    }),
  )
  await page.goto(`${ORIGIN}/`)
}
