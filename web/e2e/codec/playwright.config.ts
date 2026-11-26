import { fileURLToPath } from 'node:url'

import { defineConfig, devices } from '@playwright/test'

// No server, so none of the console flows' global setup.
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
