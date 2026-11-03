import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import { pageOf } from '@/features/users/test-data'

import { DEVICE_GROUPS_PATH, DEVICES_PATH, deviceGroupView, deviceView } from '../test-data'

const OFFICE = deviceGroupView(3, 'Office')

function listCalls(fetchSpy: Parameters<typeof requestedUrl>[0]) {
  return fetchSpy.mock.calls
    .map((_, index) => requestedUrl(fetchSpy, index))
    .filter((url) => url.pathname === DEVICES_PATH)
}

describe('devices page', () => {
  it('lists devices with status, owner, group and last seen', async () => {
    renderApp('/devices', {
      api: {
        [`GET ${DEVICES_PATH}`]: pageOf([
          deviceView(1, '100200300', {
            online: true,
            owner: { id: 5, username: 'bob' },
            device_group: { id: 3, name: 'Office' },
          }),
          deviceView(2, '400500600', { hostname: null, status: 'disabled' }),
          deviceView(3, '700800900', { hostname: 'gamma' }),
        ]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
      },
    })

    await screen.findByRole('button', { name: '100200300' })
    const table = screen.getByRole('table', { name: '设备' })
    const [, firstRow, secondRow, thirdRow] = within(table).getAllByRole('row')
    const first = within(firstRow ?? table)
    expect(first.getByRole('button', { name: '100200300' })).toBeInTheDocument()
    expect(first.getByText('在线')).toBeInTheDocument()
    expect(first.getByText('alpha')).toBeInTheDocument()
    expect(first.getByText('bob')).toBeInTheDocument()
    expect(first.getByText('Office')).toBeInTheDocument()
    expect(within(secondRow ?? table).getByText('已禁用')).toBeInTheDocument()
    expect(within(thirdRow ?? table).getByText('离线')).toBeInTheDocument()
  })

  it('filters by online status and device group through the URL', async () => {
    const { router, user, fetchSpy } = renderApp('/devices', {
      api: {
        [`GET ${DEVICES_PATH}`]: pageOf([deviceView(1, '100200300')]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([OFFICE]),
      },
    })
    await screen.findByRole('button', { name: '100200300' })

    await user.selectOptions(screen.getByRole('combobox', { name: '在线状态' }), '在线')
    await waitFor(() => {
      expect(router.state.location.search).toMatchObject({ online: true })
    })
    await user.selectOptions(await screen.findByRole('combobox', { name: '设备组' }), 'Office')
    await waitFor(() => {
      expect(router.state.location.search).toMatchObject({ online: true, device_group: 3 })
    })
    await waitFor(() => {
      const last = listCalls(fetchSpy).at(-1)
      expect(last?.searchParams.get('online')).toBe('true')
      expect(last?.searchParams.get('device_group')).toBe('3')
    })
  })

  it('reads the filters from the URL', async () => {
    const { fetchSpy } = renderApp('/devices?online=false&status=disabled&sort=-last_seen_at', {
      api: {
        [`GET ${DEVICES_PATH}`]: pageOf([]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([]),
      },
    })
    await waitFor(() => {
      expect(listCalls(fetchSpy)).not.toHaveLength(0)
    })
    const params = listCalls(fetchSpy)[0]?.searchParams
    expect(params?.get('online')).toBe('false')
    expect(params?.get('status')).toBe('disabled')
    expect(params?.get('sort')).toBe('-last_seen_at')
  })

  it('shows the way to client setup when there are no devices', async () => {
    renderApp('/devices', {
      api: {
        [`GET ${DEVICES_PATH}`]: pageOf([]),
        [`GET ${DEVICE_GROUPS_PATH}`]: pageOf([]),
      },
    })
    expect(await screen.findByRole('link', { name: '前往客户端配置' })).toBeInTheDocument()
  })
})
