import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { DeviceDetail, DeviceView } from '@/api/generated/model'
import { OWNER_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'
import { pageOf, userView } from '@/features/users/test-data'

import { ALPHA, ALPHA_PATH, deviceDetail, drawerApi } from '../test-data'

const ONE = ALPHA_PATH
const USERS = '/admin/api/v1/users'
const api = drawerApi

function detailWith(patch: Partial<DeviceDetail> = {}, device: DeviceView = ALPHA) {
  return { body: deviceDetail(device, patch) }
}

describe('device drawer: details', () => {
  it('checks again whether the browser remote is on when it opens', async () => {
    const me = { ...OWNER_ME }
    const { user, router } = renderApp('/devices', {
      api: api({ 'GET /admin/api/v1/me': () => ({ body: me }) }),
    })
    const open = await screen.findByRole('button', { name: ALPHA.rustdesk_id })
    me.web_client_enabled = true

    await user.click(open)
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })

    expect(await within(drawer).findByRole('link', { name: '浏览器连接' })).toBeInTheDocument()
    expect(router.state.location.search).toMatchObject({ device: 1 })
  })
})

describe('device drawer: editing', () => {
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
    await user.click(await screen.findByRole('option', { name: /carol/ }))
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
    await user.click(within(confirm).getByRole('button', { name: '断开连接' }))

    await waitFor(() => {
      expect(posted).toEqual({ conn_id: 9 })
    })
  })

  it('disables a device after confirming', async () => {
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

  it('deletes a device only after its ID is typed', async () => {
    const { user, router } = renderApp('/devices?device=1', {
      api: api({ [`DELETE ${ONE}`]: { status: 204 } }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.click(within(drawer).getByRole('button', { name: '删除设备' }))
    const confirm = await screen.findByRole('alertdialog', { name: '删除此设备？' })
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
