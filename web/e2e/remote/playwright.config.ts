import { fileURLToPath } from 'node:url'

import { defineConfig, devices } from '@playwright/test'

// One stack per run, so one worker. Specs are `*.remote.ts` so the console config never matches them.
const results = fileURLToPath(new URL('../../e2e-results/remote', import.meta.url))
const owner = fileURLToPath(new URL('../.auth/remote-owner.json', import.meta.url))
const slow = process.env.RD_E2E_SLOW === '1'

export default defineConfig({
  testDir: '.',
  testMatch: '*.remote.ts',
  globalSetup: './global-setup.ts',
  outputDir: `${results}/artifacts`,
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 20_000 },
  reporter: [['list'], ['html', { outputFolder: `${results}/report`, open: 'never' }]],
  use: { locale: 'en-US', trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  projects: [
    { name: 'login', testMatch: 'login.setup.remote.ts' },
    {
      name: 'chromium',
      // The @slow specs (minutes long) run alone and only on request.
      ...(slow
        ? { testMatch: ['stability.remote.ts', 'perf.remote.ts'] }
        : { testIgnore: ['login.setup.remote.ts', 'stability.remote.ts', 'perf.remote.ts'] }),
      dependencies: ['login'],
      use: { ...devices['Desktop Chrome'], storageState: owner },
    },
    {
      name: 'firefox',
      testMatch: slow
        ? 'stability.remote.ts'
        : ['relay.remote.ts', 'webrtc.remote.ts', 'paste.remote.ts'],
      dependencies: ['login'],
      use: { ...devices['Desktop Firefox'], storageState: owner },
    },
  ],
})
