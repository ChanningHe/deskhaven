import { fileURLToPath } from 'node:url'

import { signIn, waitOnline } from './support/console.ts'
import { stackInfo } from './support/env.ts'
import { test } from './support/fixtures.ts'

const OWNER_STATE = fileURLToPath(new URL('../.auth/remote-owner.json', import.meta.url))

test('the owner signs in and every client device registers', async ({ page }) => {
  await signIn(page)
  for (const device of Object.values(stackInfo().devices)) await waitOnline(page, device.id)
  await page.context().storageState({ path: OWNER_STATE })
})
