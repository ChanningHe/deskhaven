// M3-04: create a user -> add them to a group -> revoke their sessions -> the client's
// token is refused (401).
import { clientLogin, currentUser } from './support/client-api.ts'
import { expect, test } from './support/fixtures.ts'
import type { Device } from './support/rendezvous.ts'

const USER = { username: 'alice', password: 'alice e2e passphrase 1' }
const GROUP = 'Support'
/** The client alice signs in from (sessions record the device; it need not be registered). */
const LAPTOP: Device = { rustdeskId: '300000001', uuid: Buffer.from('alice-laptop-uuid') }

test('a new user in a group loses the client session an admin revokes', async ({
  page,
  serverUrl,
}) => {
  await test.step('create the user with a chosen password', async () => {
    await page.goto('/users')
    await page.getByRole('button', { name: 'New user' }).click()
    const dialog = page.getByRole('dialog', { name: 'New user' })
    await dialog.getByLabel('Username').fill(USER.username)
    await dialog.getByText('Set the initial password myself').click()
    await dialog.getByRole('textbox', { name: 'Initial password' }).fill(USER.password)
    await dialog.getByRole('button', { name: 'Create user' }).click()
    await expect(dialog).toBeHidden()
    await expect(page.getByRole('button', { name: USER.username })).toBeVisible()
  })

  await test.step('create a group and add the user', async () => {
    await page.goto('/groups')
    await page.getByRole('button', { name: 'New group' }).click()
    const dialog = page.getByRole('dialog', { name: 'New user group' })
    await dialog.getByLabel('Name').fill(GROUP)
    await dialog.getByRole('button', { name: 'Create group' }).click()
    const drawer = page.getByRole('dialog', { name: GROUP })
    await drawer.getByRole('combobox', { name: 'Find a user to add' }).fill(USER.username)
    await drawer.getByRole('option', { name: USER.username }).click()
    await expect(drawer.getByRole('listitem').filter({ hasText: USER.username })).toBeVisible()
  })

  const token = await clientLogin(serverUrl, LAPTOP, USER.username, USER.password)
  expect((await currentUser(serverUrl, LAPTOP, token)).status).toBe(200)

  await test.step('revoke the sessions from the user drawer', async () => {
    await page.goto('/users')
    await page.getByRole('button', { name: USER.username }).click()
    const drawer = page.getByRole('dialog', { name: USER.username })
    await expect(drawer.getByText(GROUP, { exact: true })).toBeVisible()
    await expect(drawer.getByText(`Client · ${LAPTOP.rustdeskId}`)).toBeVisible()
    await drawer.getByRole('button', { name: 'Revoke sessions' }).click()
    const confirm = page.getByRole('alertdialog', { name: 'Revoke all sessions?' })
    await confirm.getByRole('button', { name: 'Revoke sessions' }).click()
    await expect(drawer.getByText('No active sessions.')).toBeVisible()
  })

  const revoked = await currentUser(serverUrl, LAPTOP, token)
  expect(revoked.status).toBe(401)
})
