import type { Page } from '@playwright/test'

import { PINNED } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'

/** A harmless duration setting to edit. */
const KEY = 'audit.retention.login'

const row = (page: Page, key: string) => page.locator(`li[data-setting="${key}"]`)

async function stage(page: Page, value: string) {
  await page.goto('/settings')
  await row(page, KEY).getByRole('textbox').fill(value)
  await expect(row(page, KEY).getByText('Unsaved')).toBeVisible()
}

test('a saved setting becomes modified', async ({ page }) => {
  await page.goto('/settings')
  await expect(row(page, KEY).getByText('Default', { exact: true })).toBeVisible()

  await stage(page, '90d')
  await page.getByRole('button', { name: 'Save changes' }).click()

  await expect(page.getByText('Saved 1 setting.')).toBeVisible()
  await expect(row(page, KEY).getByText('Modified', { exact: true })).toBeVisible()
})

test('a setting pinned by the environment is read-only and names the variable', async ({
  page,
}) => {
  await page.goto('/settings')
  const pinned = row(page, PINNED.key)

  await expect(pinned.getByText('Locked by config', { exact: true })).toBeVisible()
  await expect(pinned).toContainText(`Fixed by environment variable ${PINNED.env}; read-only here.`)
  await expect(pinned.getByRole('combobox')).toBeDisabled()
})

test('saving over a concurrent change asks whose value wins', async ({ page, context }) => {
  const other = await context.newPage()
  await stage(page, '60d')
  await stage(other, '30d')

  await page.getByRole('button', { name: 'Save changes' }).click()
  await expect(page.getByText('Saved 1 setting.')).toBeVisible()
  await other.getByRole('button', { name: 'Save changes' }).click()

  const conflict = other.getByRole('alertdialog', { name: 'Changed by someone else' })
  await expect(conflict).toContainText('60d')
  await conflict.getByRole('button', { name: 'Keep my values' }).click()
  await expect(other.getByText('Saved 1 setting.')).toBeVisible()
  await expect(row(other, KEY).getByRole('textbox')).toHaveValue('30d')
})
