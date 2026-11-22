// (b) WebRTC with a 1.5.x client, and (c) the fallback: with every UDP path closed the
// session still comes up, over the relay.
import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'

test('webrtc: a 1.5 client is reached over WebRTC', async ({ page }) => {
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return

  await openFromDeviceList(page, target.id)
  await submitPassword(page)
  await expectDesktop(page, 'WebRTC')
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
