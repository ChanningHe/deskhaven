// M3-13: a user binds TOTP on "My account"; the client login then asks for the code.
import { accessToken, codeLogin, passwordLogin } from './support/client-api.ts'
import { bindTotp, createUser, submitPassword } from './support/console.ts'
import { expect, test } from './support/fixtures.ts'
import type { Device } from './support/rendezvous.ts'

const USER = { username: 'bob', password: 'bob e2e passphrase 1' }
/** An administrator set the first password, so the console asks for a new one first. */
const OWN_PASSWORD = 'bob chose this passphrase 2'
const DESKTOP: Device = { rustdeskId: '300000002', uuid: Buffer.from('bob-desktop-uuid') }

test('a user who binds TOTP themselves is asked for the code at client login', async ({
  page,
  browser,
  serverUrl,
}) => {
  await createUser(page, USER)
  // A context of its own without the owner's cookies (a new context inherits the project's
  // storageState): signing in revokes the session the browser presents.
  const bob = await browser.newContext({
    baseURL: serverUrl,
    storageState: { cookies: [], origins: [] },
  })
  const own = await bob.newPage()

  await test.step('sign in and replace the password an administrator set', async () => {
    await submitPassword(own, USER)
    await expect(own.getByText('Choose a new password')).toBeVisible()
    await own.getByLabel('Current password').fill(USER.password)
    await own.getByLabel('New password', { exact: true }).fill(OWN_PASSWORD)
    await own.getByLabel('Repeat new password').fill(OWN_PASSWORD)
    await own.getByRole('button', { name: 'Change password' }).click()
    await expect(own.getByText('Choose a new password')).toBeHidden()
  })

  const authenticator = await test.step('bind TOTP on My account', async () => {
    await own.goto('/me')
    await own.getByRole('button', { name: 'Enable two-factor authentication' }).click()
    const dialog = own.getByRole('dialog', { name: 'Enable two-factor authentication' })
    const bound = await bindTotp(own, dialog, OWN_PASSWORD)
    await expect(own.getByText('Enabled', { exact: true })).toBeVisible()
    return bound
  })
  await bob.close()

  const challenge = await passwordLogin(serverUrl, DESKTOP, USER.username, OWN_PASSWORD)
  expect(challenge.status).toBe(200)
  expect(challenge.body).toMatchObject({ type: 'email_check', tfa_type: 'tfa_check' })
  const secret = String(challenge.body.secret)
  const code = authenticator.nextCode()
  const signedIn = await codeLogin(serverUrl, DESKTOP, USER.username, secret, code)
  expect(accessToken(signedIn)).not.toBe('')
})
