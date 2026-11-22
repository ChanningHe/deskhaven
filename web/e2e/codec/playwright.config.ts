import { fileURLToPath } from 'node:url'

import { defineConfig, devices } from '@playwright/test'

// Browser checks of the remote client's media path; no server, so none of the console flows'
// global setup (e2e/README.md, "Remote client codec check").
export default defineConfig({
  testDir: '.',
  testMatch: '*.codec.ts',
  outputDir: fileURLToPath(new URL('../../e2e-results/codec', import.meta.url)),
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  timeout: 60_000,
  reporter: 'list',
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
