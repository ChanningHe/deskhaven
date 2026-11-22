// Sessions are limited per user (4 at once, 10 new ones a minute); closing the tab must
// release its session, or the fifth connection in a row is refused. Named to run last: it
// waits out the rate window first so the specs before it cannot make it hit the limit.
import { device, expectDesktop, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'

const RATE_WINDOW_MS = 61_000
const CONNECTIONS = 5

test('closing the tab releases the session', async ({ context }) => {
  test.setTimeout(RATE_WINDOW_MS + 90_000)
  const target = device('1.5')
  test.skip(target === undefined, 'no 1.5 client in this run')
  if (target === undefined) return
  await new Promise((resolve) => setTimeout(resolve, RATE_WINDOW_MS))
  for (let i = 0; i < CONNECTIONS; i += 1) {
    const page = await context.newPage()
    await page.addInitScript(() => {
      Reflect.deleteProperty(window, 'RTCPeerConnection')
    })
    await page.goto(`/remote/${target.id}`)
    await submitPassword(page)
    await expectDesktop(page, 'Relay')
    await page.close()
  }
  expect(context.pages()).toHaveLength(0)
})
