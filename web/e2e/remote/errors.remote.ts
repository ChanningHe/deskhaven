// (e) Error paths: a device that is offline, and the switch turned off.
import { device, openFromDeviceList, setWebClient } from './support/console.ts'
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
        await page.goto(`/remote/${ghost}`)
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

test('with web_client.enabled off there is no entry and the page says why', async ({ page }) => {
  const target = device('1.5') ?? device('1.4')
  test.skip(target === undefined, 'no client in this run')
  if (target === undefined) return
  await setWebClient(page, false)
  try {
    await page.goto('/devices')
    await page.getByText(target.id, { exact: true }).first().click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByRole('link', { name: 'Connect in browser' })).toHaveCount(0)

    await page.goto(`/remote/${target.id}`)
    await expect(page.getByText(/Remote control from the browser is turned off/)).toBeVisible()
  } finally {
    await setWebClient(page, true)
  }
  // And back on: the entry returns.
  await openFromDeviceList(page, target.id)
})
