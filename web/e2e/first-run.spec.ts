// M3-02: first run -> owner -> TOTP enrolment -> sign out -> sign in needs the code;
// the setup link works once. Saves the owner's session for the other projects.
import type { Page } from '@playwright/test'

import { bindTotp, signOut, submitCode, submitPassword } from './support/console.ts'
import { OWNER, OWNER_STATE, SETUP_PATH_ENV, required } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'

test.describe.configure({ mode: 'serial' })

const setupPath = () => required(SETUP_PATH_ENV)

async function createOwner(page: Page) {
  await page.goto(setupPath())
  await page.getByLabel('Username').fill(OWNER.username)
  await page.getByLabel('Password', { exact: true }).fill(OWNER.password)
  await page.getByLabel('Repeat password').fill(OWNER.password)
  await page.getByRole('button', { name: 'Create owner' }).click()
  await page.getByRole('button', { name: 'Next' }).click()
  await page.getByRole('button', { name: 'Open the console' }).click()
}

test('first run creates the owner, who must bind TOTP and then sign in with it', async ({
  page,
}) => {
  await page.goto('/login')
  await expect(page.getByText(/This server has no owner yet/)).toBeVisible()

  await createOwner(page)
  // `auth.tfa.enforce` = admins (pinned): the owner must enrol before anything else.
  await expect(page).toHaveURL(/\/account\/two-factor$/)
  const authenticator = await bindTotp(page, page, OWNER.password)
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible()

  await signOut(page)
  await submitPassword(page, OWNER)
  await submitCode(page, authenticator)
  await expect(page.getByRole('heading', { name: 'Overview' })).toBeVisible()

  await page.context().storageState({ path: OWNER_STATE })
})

test('the setup link works only once', async ({ page }) => {
  await page.goto(setupPath())
  await expect(page.getByText('This server is already set up.')).toBeVisible()

  const token = new URLSearchParams(setupPath().split('#')[1]).get('token')
  const reply = await page.evaluate(async (setupToken) => {
    const response = await fetch('/admin/api/v1/setup', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Deskhaven-CSRF': 'anonymous' },
      body: JSON.stringify({ token: setupToken, username: 'second', password: 'x'.repeat(16) }),
    })
    return { status: response.status, body: await response.json() }
  }, token)

  expect(reply.status).toBe(409)
  expect(reply.body).toMatchObject({ error: { code: 'setup_completed' } })
})
