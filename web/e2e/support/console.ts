// Console steps more than one flow takes.
import type { Locator, Page } from '@playwright/test'
import { expect } from '@playwright/test'

import { Authenticator } from './totp.ts'

export interface Account {
  username: string
  password: string
}

/** Users page, "New user" with a chosen password (no forced change at first sign-in). */
export async function createUser(page: Page, user: Account) {
  await page.goto('/users')
  await page.getByRole('button', { name: 'New user' }).click()
  const dialog = page.getByRole('dialog', { name: 'New user' })
  await dialog.getByLabel('Username').fill(user.username)
  await dialog.getByText('Set the initial password myself').click()
  await dialog.getByRole('textbox', { name: 'Initial password' }).fill(user.password)
  await dialog.getByRole('button', { name: 'Create user' }).click()
  await expect(dialog).toBeHidden()
  await expect(page.getByRole('button', { name: user.username })).toBeVisible()
}

/**
 * A TOTP enrolment form inside `form` (the forced enrolment page or the /me dialog): read
 * the seed like an authenticator app, confirm with the password and a code, save the
 * recovery codes. The authenticator for later sign-ins.
 */
export async function bindTotp(page: Page, form: Page | Locator, password: string) {
  const seed = form.locator('p:has-text("Or enter this key manually:") + code')
  const authenticator = new Authenticator(await seed.innerText())
  await form.getByLabel('Current password').fill(password)
  await form.getByLabel('Code from the app').fill(authenticator.nextCode())
  await form.getByRole('button', { name: 'Enable' }).click()
  const recovery = page.getByRole('dialog', { name: 'Recovery codes' })
  await recovery.getByText('I have saved it').click()
  await recovery.getByRole('button', { name: 'Done' }).click()
  await expect(recovery).toBeHidden()
  return authenticator
}

/** The login page's password step (a TOTP step may follow). */
export async function submitPassword(page: Page, account: Account) {
  await page.goto('/login')
  await page.getByLabel('Username').fill(account.username)
  await page.getByLabel('Password').fill(account.password)
  await page.getByRole('button', { name: 'Sign in' }).click()
}

/** The login page's TOTP step, with the next code of `authenticator`. */
export async function submitCode(page: Page, authenticator: Authenticator) {
  await expect(page.getByText('Two-factor authentication')).toBeVisible()
  await page.getByLabel('Code', { exact: true }).fill(authenticator.nextCode())
  await page.getByRole('button', { name: 'Verify' }).click()
}

export async function signOut(page: Page) {
  await page.getByRole('button', { name: 'Account menu' }).click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()
  await expect(page).toHaveURL(/\/login/)
}
