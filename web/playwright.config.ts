import { defineConfig, devices } from '@playwright/test'

import { OWNER_STATE, resultsDir } from './e2e/support/env.ts'

// End-to-end flows of the console against the real binary (e2e/README.md). One server per
// run and flows that build on each other (the owner first), so one worker, in order.
const results = resultsDir()

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  outputDir: `${results}/artifacts`,
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: [['list'], ['html', { outputFolder: `${results}/report`, open: 'never' }]],
  use: {
    ...devices['Desktop Chrome'],
    locale: 'en-US',
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'first-run', testMatch: 'first-run.spec.ts' },
    {
      name: 'console',
      testIgnore: 'first-run.spec.ts',
      dependencies: ['first-run'],
      use: { storageState: OWNER_STATE },
    },
  ],
})
