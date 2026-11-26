import { openFromLink } from './support/console.ts'
import { stackInfo } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'

/** The signal server's peer timeout, with room: the ghost device registered at the start. */
const OFFLINE_WITHIN_MS = 150_000

test('an offline device shows the device_offline message', async ({ page }) => {
  test.setTimeout(OFFLINE_WITHIN_MS + 30_000)
  const ghost = stackInfo().ghostId
  // The device registered once and never again. Until the signal server drops it, the
  // answer is "no answer" (504); then "offline" (409). Both are real answers: wait for the latter.
  await expect
    .poll(
      async () => {
        await openFromLink(page, ghost)
        const message = page
          .getByRole('alert')
          .or(page.getByText(/The device (is offline|did not answer)/))
        await message.first().waitFor({ timeout: 30_000 })
        return message.first().innerText()
      },
      { timeout: OFFLINE_WITHIN_MS },
    )
    .toContain('The device is offline. Start it, or wait until it reconnects to the server.')
})
