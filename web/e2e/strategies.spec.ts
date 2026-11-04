// M3-07: create a strategy -> assign it to a device group -> the preview shows the merged
// result -> the device's next heartbeat delivers it.
import { heartbeat } from './support/client-api.ts'
import { SIGNAL_PORT_ENV, required } from './support/env.ts'
import { expect, test } from './support/fixtures.ts'
import { registerDevice } from './support/rendezvous.ts'
import type { Device } from './support/rendezvous.ts'

const DEVICE: Device = { rustdeskId: '512345678', uuid: Buffer.from('e2e-lab-device-uuid') }
const DEVICE_GROUP = 'Lab'
const STRATEGY = 'Lab policy'
/** A catalog option (bool) and the value the strategy sets. */
const OPTION = { label: 'Clipboard', key: 'enable-clipboard', choice: 'Off', value: 'N' }

test('a strategy assigned to a device group reaches its device', async ({ page, serverUrl }) => {
  await registerDevice(Number(required(SIGNAL_PORT_ENV)), DEVICE)

  await test.step('put the device into a new device group', async () => {
    await page.goto('/groups')
    await page.getByRole('tab', { name: 'Device groups' }).click()
    await page.getByRole('button', { name: 'New device group' }).click()
    const dialog = page.getByRole('dialog', { name: 'New device group' })
    await dialog.getByLabel('Name').fill(DEVICE_GROUP)
    await dialog.getByRole('button', { name: 'Create device group' }).click()
    await expect(page.getByRole('dialog', { name: DEVICE_GROUP })).toBeVisible()

    await page.goto('/devices')
    await page.getByRole('button', { name: DEVICE.rustdeskId }).click()
    const drawer = page.getByRole('dialog', { name: DEVICE.rustdeskId })
    await drawer.getByLabel('Device group').selectOption({ label: DEVICE_GROUP })
    await drawer.getByRole('button', { name: 'Save changes' }).click()
    await expect(drawer.getByText('Saved', { exact: true })).toBeVisible()
  })

  await test.step('create the strategy and assign it to the device group', async () => {
    await page.goto('/strategies')
    await page.getByRole('button', { name: 'New strategy' }).click()
    const dialog = page.getByRole('dialog', { name: 'New strategy' })
    await dialog.getByLabel('Name').fill(STRATEGY)
    await dialog.getByRole('button', { name: 'Create strategy' }).click()
    const drawer = page.getByRole('dialog', { name: STRATEGY })
    await drawer.getByLabel(OPTION.label, { exact: true }).selectOption({ label: OPTION.choice })
    await drawer.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(drawer.getByText('Saved', { exact: true })).toBeVisible()

    await drawer.getByRole('tab', { name: 'Assignments' }).click()
    await drawer.getByLabel('Target type').selectOption({ label: 'Device group' })
    await drawer.getByLabel('Device group', { exact: true }).selectOption({ label: DEVICE_GROUP })
    await drawer.getByRole('button', { name: 'Assign', exact: true }).click()
    await expect(drawer.getByText(`Device group: ${DEVICE_GROUP}`)).toBeVisible()
  })

  await test.step('the preview merges it for the device', async () => {
    const drawer = page.getByRole('dialog', { name: STRATEGY })
    await drawer.getByRole('tab', { name: 'Preview' }).click()
    await drawer.getByLabel('Device', { exact: true }).selectOption({ label: DEVICE.rustdeskId })
    const merged = drawer.getByRole('row', { name: OPTION.key })
    await expect(merged).toContainText(OPTION.value)
    await expect(merged).toContainText(`Device group: ${STRATEGY}`)
  })

  const reply = await heartbeat(serverUrl, DEVICE)
  expect(reply.status).toBe(200)
  expect(reply.body).toMatchObject({ strategy: { config_options: { [OPTION.key]: OPTION.value } } })
})
