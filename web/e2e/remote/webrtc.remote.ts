import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'
import { xclock } from './support/x11.ts'

test('webrtc: a 1.5 client is reached over WebRTC', async ({ page }) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'WebRTC')
})

test('relay only: switching mid-session moves a 1.5 client to the relay', async ({ page }) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'WebRTC')
  await page.getByRole('button', { name: 'Display' }).click()
  await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
  const stat = (name: string) => page.locator(`[data-stat="${name}"]`)
  await expect(stat('transport')).toHaveAttribute('data-value', 'webrtc')

  const clock = xclock(target)
  try {
    await page.getByRole('button', { name: 'Display' }).click()
    await page.getByRole('menuitemradio', { name: 'Relay only' }).click()
    await expect(stat('transport')).toHaveAttribute('data-value', 'relay')
    await expectDesktop(page, 'Relay')
    await expect(page.getByRole('textbox', { name: 'Password' })).toHaveCount(0)
    await expect
      .poll(async () => Number(await stat('fps').getAttribute('data-value')))
      .toBeGreaterThan(0)
  } finally {
    clock.stop()
  }
})

test('fallback: without UDP the 1.5 client is reached over the relay', async ({ page }) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return
  // Relay-only ICE with no TURN server: the browser gathers no candidate at all, so the
  // data channel can never open and the race falls back. Done in the page, so it holds
  // in every browser; it also overrides `setConfiguration`, which the client calls later.
  await page.addInitScript(() => {
    const Native = window.RTCPeerConnection
    const blocked = (config?: RTCConfiguration): RTCConfiguration => ({
      ...config,
      iceServers: [],
      iceTransportPolicy: 'relay',
    })
    class NoUdp extends Native {
      constructor(config?: RTCConfiguration) {
        super(blocked(config))
        Reflect.set(window, '__noUdpPeers', Number(Reflect.get(window, '__noUdpPeers') ?? 0) + 1)
      }
      override setConfiguration(config?: RTCConfiguration) {
        super.setConfiguration(blocked(config))
      }
    }
    window.RTCPeerConnection = NoUdp
  })

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'Relay')
  // The WebRTC attempt was made (and lost), not skipped.
  expect(
    await page.evaluate(() => Number(Reflect.get(window, '__noUdpPeers') ?? 0)),
  ).toBeGreaterThan(0)
})
