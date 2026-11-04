import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { DeviceDetail, DeviceView } from '@/api/generated/model'
import { renderApp } from '@/test/render-app'
import { pageOf, userView } from '@/features/users/test-data'

import { ALPHA, ALPHA_PATH, deviceDetail, drawerApi } from '../test-data'

const ONE = ALPHA_PATH
const USERS = '/admin/api/v1/users'
const api = drawerApi

function detailWith(patch: Partial<DeviceDetail> = {}, device: DeviceView = ALPHA) {
  return { body: deviceDetail(device, patch) }
}

async function openDrawer(path = '/devices?device=1') {
  const app = renderApp(path, { api: api() })
  return { ...app, drawer: await screen.findByRole('dialog', { name: 'alpha' }) }
}

describe('device drawer: details', () => {
  it('labels what the device reported about itself', async () => {
    const { drawer } = await openDrawer()
    const reported = within(drawer).getByRole('region', { name: '设备自报' })
    expect(within(reported).getByText('Intel, 3.2GHz, 8/4 cores')).toBeInTheDocument()
    expect(within(reported).getByText('15.5GB')).toBeInTheDocument()
    expect(within(reported).getByText('1.4.1')).toBeInTheDocument()
    expect(within(drawer).getByText(/服务器无法核实/)).toBeInTheDocument()
  })

  it('shows the effective strategy with the layer each option comes from', async () => {
    const layer = (scope: 'global' | 'device_group', id: number, name: string) => ({
      scope,
      target_id: scope === 'global' ? null : id,
      strategy_id: id,
      strategy_name: name,
    })
    const strategy = {
      modified_at: 1,
      layers: [layer('global', 1, 'base'), layer('device_group', 2, 'night')],
      options: [
        { key: 'lang', value: 'en', redacted: false, source: layer('global', 1, 'base') },
        {
          key: 'theme',
          value: 'light',
          redacted: false,
          source: layer('device_group', 2, 'night'),
        },
        { key: 'wallpaper', value: '', redacted: false, source: layer('device_group', 2, 'night') },
      ],
    }
    renderApp('/devices?device=1', { api: api({}, detailWith({ strategy })) })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const section = within(drawer).getByRole('region', { name: '生效策略' })
    const theme = within(section).getByRole('row', { name: /theme/ })
    expect(within(theme).getByText('light')).toBeInTheDocument()
    expect(within(theme).getByText(/设备组：night/)).toBeInTheDocument()
    expect(
      within(within(section).getByRole('row', { name: /lang/ })).getByText(/全局：base/),
    ).toBeInTheDocument()
    expect(within(section).getByRole('row', { name: /wallpaper/ })).toHaveTextContent('空值')
  })

  it('says so when no strategy applies and when nothing was recorded', async () => {
    const { drawer } = await openDrawer()
    expect(within(drawer).getByText('没有策略作用于此设备。')).toBeInTheDocument()
    expect(within(drawer).getByText('当前没有连接。')).toBeInTheDocument()
    expect(within(drawer).getByText('暂无连接记录。')).toBeInTheDocument()
  })

  it('lists recent connections', async () => {
    const recent = [
      {
        guid: 'g1',
        conn_id: 7,
        peer_rustdesk_id: '555',
        peer_name: 'laptop',
        controller_user_id: null,
        conn_type: 1,
        ip: '198.51.100.4',
        started_at: 1_700_000_000_000,
        ended_at: null,
      },
    ]
    renderApp('/devices?device=1', { api: api({}, detailWith({ recent_connections: recent })) })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const section = within(drawer).getByRole('region', { name: '最近连接' })
    expect(within(section).getByText(/文件传输/)).toBeInTheDocument()
    expect(within(section).getByText(/laptop/)).toBeInTheDocument()
    expect(within(section).getByText('进行中')).toBeInTheDocument()
  })
})

describe('device drawer: editing', () => {
  it('assigns a device group and a note', async () => {
    let patched: unknown
    const { user } = renderApp('/devices?device=1', {
      api: api({
        [`PATCH ${ONE}`]: ({ init }: { init: RequestInit }) => {
          patched = JSON.parse(init.body as string)
          return { body: { ...ALPHA, note: 'rack 4' } }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const save = within(drawer).getByRole('button', { name: '保存修改' })
    expect(save).toBeDisabled()

    await user.selectOptions(within(drawer).getByLabelText('设备组'), 'Office')
    const note = within(drawer).getByLabelText('备注')
    await user.clear(note)
    await user.type(note, 'rack 4')
    await user.click(save)

    await waitFor(() => {
      expect(patched).toEqual({ device_group_id: 3, note: 'rack 4' })
    })
  })

  it('finds a user to make owner, and removes the owner', async () => {
    const bodies: unknown[] = []
    const { user } = renderApp('/devices?device=1', {
      api: api({
        [`PATCH ${ONE}`]: ({ init }: { init: RequestInit }) => {
          bodies.push(JSON.parse(init.body as string))
          return { body: ALPHA }
        },
        [`GET ${USERS}`]: pageOf([userView(6, 'carol')]),
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    expect(within(drawer).getByText('bob')).toBeInTheDocument()

    await user.click(within(drawer).getByRole('button', { name: '移除所有者' }))
    await user.type(within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' }), 'car')
    await user.click(await within(drawer).findByRole('option', { name: /carol/ }))
    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ owner_user_id: 6 }])
    })

    await user.click(within(drawer).getByRole('button', { name: '移除所有者' }))
    await user.click(within(drawer).getByRole('button', { name: '保存修改' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ owner_user_id: 6 }, { owner_user_id: null }])
    })
  })
})

describe('device drawer: actions', () => {
  it('disconnects a connection after confirming, once the device reports it', async () => {
    let posted: unknown
    const { user } = renderApp('/devices?device=1', {
      api: api(
        {
          [`POST ${ONE}/disconnect`]: ({ init }: { init: RequestInit }) => {
            posted = JSON.parse(init.body as string)
            return { status: 202, body: { ttl_seconds: 60 } }
          },
        },
        detailWith({ connections: [7, 9] }),
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.click(within(drawer).getByRole('button', { name: '断开连接 9' }))
    const confirm = await screen.findByRole('alertdialog', { name: '断开此连接？' })
    expect(within(confirm).getByText(/60 秒/)).toBeInTheDocument()
    await user.click(within(confirm).getByRole('button', { name: '断开连接' }))

    await waitFor(() => {
      expect(posted).toEqual({ conn_id: 9 })
    })
    expect(await within(drawer).findByText(/已请求断开连接 9/)).toBeInTheDocument()
  })

  it('disables after confirming and enables a disabled device at once', async () => {
    const bodies: unknown[] = []
    const patch = ({ init }: { init: RequestInit }) => {
      bodies.push(JSON.parse(init.body as string))
      return { body: ALPHA }
    }
    const { user } = renderApp('/devices?device=1', { api: api({ [`PATCH ${ONE}`]: patch }) })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.click(within(drawer).getByRole('button', { name: '禁用设备' }))
    const confirm = await screen.findByRole('alertdialog', { name: '禁用此设备？' })
    await user.click(within(confirm).getByRole('button', { name: '禁用设备' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ status: 'disabled' }])
    })
  })

  it('enables a disabled device without asking', async () => {
    const bodies: unknown[] = []
    const disabled = { ...ALPHA, status: 'disabled' as const }
    const { user } = renderApp('/devices?device=1', {
      api: api(
        {
          [`PATCH ${ONE}`]: ({ init }: { init: RequestInit }) => {
            bodies.push(JSON.parse(init.body as string))
            return { body: ALPHA }
          },
        },
        detailWith({}, disabled),
      ),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.click(within(drawer).getByRole('button', { name: '启用设备' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ status: 'active' }])
    })
  })

  it('explains what deleting a TCP-mode device means and asks for the ID', async () => {
    const { user, router } = renderApp('/devices?device=1', {
      api: api({ [`DELETE ${ONE}`]: { status: 204 } }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.click(within(drawer).getByRole('button', { name: '删除设备' }))
    const confirm = await screen.findByRole('alertdialog', { name: '删除此设备？' })
    expect(confirm).toHaveTextContent('TCP 模式')
    expect(confirm).toHaveTextContent('重新连接信令服务器')
    const remove = within(confirm).getByRole('button', { name: '删除设备' })
    expect(remove).toBeDisabled()

    await user.type(within(confirm).getByRole('textbox'), '100200300')
    await user.click(remove)
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
    expect(screen.queryByRole('dialog', { name: 'alpha' })).not.toBeInTheDocument()
  })
})
