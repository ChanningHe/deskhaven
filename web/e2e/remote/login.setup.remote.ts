// Signs the owner in, turns the browser client on, waits for every device to be online, and
// saves the session for the other projects.
import { fileURLToPath } from 'node:url'

import { setWebClient, signIn, waitOnline } from './support/console.ts'
import { stackInfo } from './support/env.ts'
import { test } from './support/fixtures.ts'

const OWNER_STATE = fileURLToPath(new URL('../.auth/remote-owner.json', import.meta.url))

test('the owner signs in and every client device registers', async ({ page }) => {
  await signIn(page)
  await setWebClient(page, true)
  for (const device of Object.values(stackInfo().devices)) await waitOnline(page, device.id)
  await page.context().storageState({ path: OWNER_STATE })
})
