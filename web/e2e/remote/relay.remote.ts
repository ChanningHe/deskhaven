// The browser has no RTCPeerConnection here, so the relay is the only path.
import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import type { ClientKey } from './support/env.ts'
import { test } from './support/fixtures.ts'

const VERSIONS: ClientKey[] = ['1.4', '1.5']

for (const key of VERSIONS) {
  test(`relay: ${key} client shows its desktop`, async ({ page }) => {
    const target = device(key)
    test.skip(target === undefined, `no ${key} client in this run`)
    if (target === undefined) return
    await page.addInitScript(() => {
      Reflect.deleteProperty(window, 'RTCPeerConnection')
    })

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    await expectDesktop(page, 'Relay')
  })
}
