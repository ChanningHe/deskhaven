import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderApp } from '@/test/render-app'
import { DEVICE_GROUPS_PATH, deviceGroupView } from '@/features/devices/test-data'
import { pageOf } from '@/features/users/test-data'

const USER_GROUPS = '/admin/api/v1/groups'
const OFFICE = deviceGroupView(3, 'Office', { device_count: 4, note: 'HQ floor' })

describe('device groups tab', () => {
  it('lists device groups with their device counts', async () => {
    renderApp('/groups?tab=devices', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([
          OFFICE,
          deviceGroupView(4, 'fleet', { managed_by: 'config' }),
        ]),
      },
    })
    await screen.findByRole('button', { name: 'Office' })
    const table = screen.getByRole('table', { name: '设备组' })
    const [, office, fleet] = within(table).getAllByRole('row')
    expect(within(office ?? table).getByRole('button', { name: 'Office' })).toBeInTheDocument()
    expect(within(office ?? table).getByText('4')).toBeInTheDocument()
    expect(within(fleet ?? table).getByText('配置锁定')).toBeInTheDocument()
  })

  it('switches between the user group and device group tabs through the URL', async () => {
    const { router, user } = renderApp('/groups', {
      api: {
        [`GET ${USER_GROUPS}`]: pageOf([]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
      },
    })
    const devices = await screen.findByRole('tab', { name: '设备组' })
    expect(screen.getByRole('tab', { name: '用户组', selected: true })).toBeInTheDocument()

    await user.click(devices)
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'devices' })
    })
    expect(await screen.findByRole('button', { name: 'Office' })).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: '用户组' }))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
  })

  it('creates a device group and opens it', async () => {
    let posted: unknown
    const created = deviceGroupView(7, 'Lab')
    const { router, user } = renderApp('/groups?tab=devices', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
        [`POST ${DEVICE_GROUPS_PATH}`]: ({ init }) => {
          posted = JSON.parse(init.body as string)
          return { status: 201, body: created }
        },
        [`GET ${DEVICE_GROUPS_PATH}/7`]: { body: created },
      },
    })
    await user.click(await screen.findByRole('button', { name: '新建设备组' }))
    const dialog = await screen.findByRole('dialog', { name: '新建设备组' })
    await user.type(within(dialog).getByLabelText('名称'), 'Lab')
    await user.click(within(dialog).getByRole('button', { name: '创建设备组' }))

    expect(await screen.findByRole('dialog', { name: 'Lab' })).toBeInTheDocument()
    expect(posted).toEqual({ name: 'Lab', note: '' })
    expect(router.state.location.search).toEqual({ tab: 'devices', group: 7 })
  })

  it('renames a group, links to its devices and deletes it after typing its name', async () => {
    let patched: unknown
    const { router, user } = renderApp('/groups?tab=devices&group=3', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
        [`GET ${DEVICE_GROUPS_PATH}/3`]: { body: OFFICE },
        [`PATCH ${DEVICE_GROUPS_PATH}/3`]: ({ init }) => {
          patched = JSON.parse(init.body as string)
          return { body: { ...OFFICE, name: 'HQ' } }
        },
        [`DELETE ${DEVICE_GROUPS_PATH}/3`]: { status: 204 },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'Office' })
    expect(within(drawer).getByText('4 台设备')).toBeInTheDocument()
    expect(within(drawer).getByRole('link', { name: '查看这些设备' })).toHaveAttribute(
      'href',
      '/devices?device_group=3',
    )

    const name = within(drawer).getByLabelText('名称')
    await user.clear(name)
    await user.type(name, 'HQ')
    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(patched).toEqual({ name: 'HQ' })
    })

    await user.click(within(drawer).getByRole('button', { name: '删除设备组' }))
    const confirm = await screen.findByRole('alertdialog', { name: '删除此设备组？' })
    await user.type(within(confirm).getByRole('textbox'), 'Office')
    await user.click(within(confirm).getByRole('button', { name: '删除设备组' }))
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ tab: 'devices' })
    })
  })

  it('shows a config-managed group read-only', async () => {
    const fleet = deviceGroupView(4, 'fleet', { managed_by: 'config' })
    renderApp('/groups?tab=devices&group=4', {
      api: {
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([fleet]),
        [`GET ${DEVICE_GROUPS_PATH}/4`]: { body: fleet },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'fleet' })
    expect(within(drawer).getByText('由配置文件定义，在此处只读。')).toBeInTheDocument()
    expect(within(drawer).queryByRole('button', { name: '删除设备组' })).not.toBeInTheDocument()
  })
})
