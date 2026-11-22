import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { callsTo, requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import type { RenderAppOptions } from '@/test/render-app'

import { account, ACCOUNT_PATH, book, BOOKS_PATH, device, DEVICES_PATH } from '../test-data'

function open(api: NonNullable<RenderAppOptions['api']>) {
  return renderApp('/me', {
    me: USER_ME,
    api: { [`GET ${ACCOUNT_PATH}`]: { body: account() }, ...api },
  })
}

describe('My account: devices', () => {
  it('lists the owned devices with their online state, device text isolated', async () => {
    const page = {
      items: [
        device('100000001', { online: true, hostname: 'office-pc' }),
        device('100000002', { hostname: null, status: 'disabled' }),
      ],
      total: 2,
      page: 1,
      page_size: 200,
    }
    const { user: actor, fetchSpy } = open({ [`GET ${DEVICES_PATH}`]: { body: page } })

    await actor.click(await screen.findByRole('tab', { name: '设备' }))

    const row = (await screen.findByText('100000001')).closest('tr') as HTMLElement
    expect(within(row).getByText('在线')).toBeInTheDocument()
    expect(within(row).getByText('office-pc').closest('bdi')).not.toBeNull()
    const other = screen.getByText('100000002').closest('tr') as HTMLElement
    expect(within(other).getByText('已禁用')).toBeInTheDocument()
    expect(screen.getByText(/由设备自行上报/)).toBeInTheDocument()
    const urls = fetchSpy.mock.calls.map((_, index) => requestedUrl(fetchSpy, index))
    const listed = urls.find((url) => url.pathname === DEVICES_PATH)
    expect(listed?.searchParams.get('page_size')).toBe('200')
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

  it('says when no device is assigned to the user', async () => {
    const { user: actor } = open({
      [`GET ${DEVICES_PATH}`]: { body: { items: [], total: 0, page: 1, page_size: 200 } },
    })

    await actor.click(await screen.findByRole('tab', { name: '设备' }))

    expect(await screen.findByText('还没有分配给你的设备')).toBeInTheDocument()
  })
})

describe('My account: address books', () => {
  it('lists the personal book and the shared ones with the rule the user holds', async () => {
    const { user: actor } = open({
      [`GET ${BOOKS_PATH}`]: {
        body: {
          items: [
            book('My address book', { kind: 'personal', rule: 3 }),
            book('Team', { owner: 'bob', rule: 1 }),
            book('Ops', { owner: null, rule: 2 }),
          ],
          shared_total: 7,
        },
      },
    })

    await actor.click(await screen.findByRole('tab', { name: '地址簿' }))

    const team = (await screen.findByText('Team')).closest('tr') as HTMLElement
    expect(within(team).getByText('共享')).toBeInTheDocument()
    expect(within(team).getByText('只读')).toBeInTheDocument()
    expect(within(team).getByText('bob')).toBeInTheDocument()
    const personal = screen.getByText('My address book').closest('tr') as HTMLElement
    expect(within(personal).getByText('个人')).toBeInTheDocument()
    expect(within(personal).getByText('完全控制')).toBeInTheDocument()
    const ops = screen.getByText('Ops').closest('tr') as HTMLElement
    expect(within(ops).getByText('读写')).toBeInTheDocument()
    expect(screen.getByText(/在客户端中编辑/)).toBeInTheDocument()
    expect(screen.getByText('仅显示 2 / 7 个共享地址簿。')).toBeInTheDocument()
  })
})
