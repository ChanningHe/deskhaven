import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import type { RenderAppOptions } from '@/test/render-app'

import { account, ACCOUNT_PATH, device, DEVICES_PATH } from '../test-data'

function open(api: NonNullable<RenderAppOptions['api']>) {
  return renderApp('/me', {
    me: USER_ME,
    api: { [`GET ${ACCOUNT_PATH}`]: { body: account() }, ...api },
  })
}

describe('My account: devices', () => {
  it('isolates device-supplied text in the owned devices list', async () => {
    const page = {
      items: [
        device('100000001', { online: true, hostname: 'office-pc' }),
        device('100000002', { hostname: null, status: 'disabled' }),
      ],
      total: 2,
      page: 1,
      page_size: 200,
    }
    const { user: actor } = open({ [`GET ${DEVICES_PATH}`]: { body: page } })

    await actor.click(await screen.findByRole('tab', { name: '设备' }))

    expect((await screen.findByText('office-pc')).closest('bdi')).not.toBeNull()
  })

  it('checks again whether the browser remote is on when the tab opens', async () => {
    const me = { ...USER_ME }
    const { user: actor, fetchSpy } = open({
      'GET /admin/api/v1/me': () => ({ body: me }),
      [`GET ${DEVICES_PATH}`]: {
        body: { items: [device('100000001', { online: true })], total: 1, page: 1, page_size: 200 },
      },
    })
    await screen.findByRole('tab', { name: '设备' })
    me.web_client_enabled = true

    await actor.click(screen.getByRole('tab', { name: '设备' }))

    expect(await screen.findByRole('link', { name: '浏览器连接' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, '/admin/api/v1/me')).toBe(2)
  })
})
