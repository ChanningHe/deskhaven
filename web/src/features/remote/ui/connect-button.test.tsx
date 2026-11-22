import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ALPHA, deviceDetail, drawerApi } from '@/features/devices/test-data'
import { ACCOUNT_PATH, DEVICES_PATH, account, device } from '@/features/me/test-data'
import { OWNER_ME, USER_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

const CONNECT = '浏览器连接'

function myDevices(enabled: boolean) {
  const page = {
    items: [
      device('100000001', { online: true }),
      device('100000002', { online: false }),
      device('100000003', { online: true, status: 'disabled' }),
    ],
    total: 3,
    page: 1,
    page_size: 200,
  }
  return renderApp('/me', {
    me: { ...USER_ME, web_client_enabled: enabled },
    api: { [`GET ${ACCOUNT_PATH}`]: { body: account() }, [`GET ${DEVICES_PATH}`]: { body: page } },
  })
}

describe('browser connect entry: my devices', () => {
  it('links an online device to its remote page and greys out the others with the reason', async () => {
    const { user } = myDevices(true)
    await user.click(await screen.findByRole('tab', { name: '设备' }))

    const online = (await screen.findByText('100000001')).closest('tr') as HTMLElement
    expect(within(online).getByRole('link', { name: CONNECT })).toHaveAttribute(
      'href',
      '/remote/100000001',
    )
    const offline = screen.getByText('100000002').closest('tr') as HTMLElement
    const greyed = within(offline).getByRole('button', { name: CONNECT })
    expect(greyed).toBeDisabled()
    const hint = within(offline).getByText(/设备离线/)
    expect(greyed.parentElement).toHaveAttribute('aria-describedby', hint.id)
    const disabled = screen.getByText('100000003').closest('tr') as HTMLElement
    expect(within(disabled).getByRole('button', { name: CONNECT })).toBeDisabled()
    expect(within(disabled).getByText(/设备已停用/)).toBeInTheDocument()
  })

  it('is absent while the server has browser remote control off', async () => {
    const { user } = myDevices(false)
    await user.click(await screen.findByRole('tab', { name: '设备' }))
    await screen.findByText('100000001')
    expect(screen.queryByText(CONNECT)).toBeNull()
    expect(screen.queryByRole('columnheader', { name: '操作' })).toBeNull()
  })
})

describe('browser connect entry: device drawer', () => {
  it('offers the connection among the device actions when enabled', async () => {
    renderApp('/devices?device=1', {
      me: { ...OWNER_ME, web_client_enabled: true },
      api: drawerApi({}, { body: deviceDetail(ALPHA) }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    const actions = within(drawer).getByRole('region', { name: '操作' })
    expect(within(actions).getByRole('link', { name: CONNECT })).toHaveAttribute(
      'href',
      `/remote/${ALPHA.rustdesk_id}`,
    )
  })

  it('leaves the device actions alone when disabled', async () => {
    renderApp('/devices?device=1', { api: drawerApi() })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    expect(within(drawer).queryByText(CONNECT)).toBeNull()
  })
})
