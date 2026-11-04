import { act, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { getGetOverviewQueryKey } from '@/api/generated/overview/overview'
import { ADMIN_ME, USER_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { OVERVIEW_PATH, overviewView } from '../test-data'

async function openOverview(view = overviewView()) {
  const app = renderApp('/', { me: ADMIN_ME, api: { [`GET ${OVERVIEW_PATH}`]: { body: view } } })
  await screen.findByText('在线设备')
  return app
}

function tile(label: string): HTMLElement {
  const root = screen.getByText(label).parentElement
  if (!root) throw new Error(`no tile ${label}`)
  return root
}

describe('overview page', () => {
  it('shows the counts as tiles, with the online share as a meter', async () => {
    await openOverview()

    expect(within(tile('在线设备')).getByText('12 / 40')).toBeInTheDocument()
    expect(within(tile('在线设备')).getByRole('meter', { name: '在线设备占比' })).toHaveAttribute(
      'aria-valuenow',
      '12',
    )
    expect(within(tile('活跃连接')).getByText('3')).toBeInTheDocument()
    expect(within(tile('连接数')).getByText('1,234')).toBeInTheDocument()
    expect(within(tile('登录失败')).getByText('7')).toBeInTheDocument()
    expect(screen.getAllByText('最近 24 小时')).toHaveLength(2)
  })

  it('draws the version distribution as labelled bars and says it is self-reported', async () => {
    await openOverview()

    const chart = screen.getByRole('region', { name: '客户端版本' })
    expect(within(chart).getByText(/由设备自行上报/)).toBeInTheDocument()
    const rows = within(chart).getAllByRole('listitem')
    expect(rows.map((row) => row.textContent)).toEqual(['1.4.130', '1.3.06', '其他版本3', '未知1'])
  })

  it('shows hostile device text as text, isolated in bdi', async () => {
    const hostile = '<img src=x onerror=alert(1)>'
    await openOverview(
      overviewView({
        client_versions: { versions: [{ version: hostile, count: 2 }], other: 0, unknown: 0 },
      }),
    )

    const chart = screen.getByRole('region', { name: '客户端版本' })
    const version = within(chart).getByText(hostile)
    expect(version.tagName).toBe('BDI')
    expect(chart.querySelector('img')).toBeNull()
  })

  it('lists the latest alarms with their device and details isolated, linking to the audit', async () => {
    await openOverview()

    const alarms = screen.getByRole('region', { name: '最近告警' })
    const items = within(alarms).getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(within(items[0] ?? alarms).getByText('100200300').tagName).toBe('BDI')
    expect(within(items[0] ?? alarms).getByText(/name=<b>mallory<\/b>/).tagName).toBe('BDI')
    expect(alarms.querySelector('b')).toBeNull()
    expect(within(alarms).getByRole('link', { name: '查看全部告警' })).toHaveAttribute(
      'href',
      '/audit?tab=alarms',
    )
  })

  it('says so when there are no alarms and no versions', async () => {
    await openOverview(
      overviewView({
        recent_alarms: [],
        devices: { online: 0, total: 0 },
        client_versions: { versions: [], other: 0, unknown: 0 },
      }),
    )

    expect(screen.getByText('暂无告警。')).toBeInTheDocument()
    expect(screen.getByText('尚无设备上报版本。')).toBeInTheDocument()
    expect(within(tile('在线设备')).getByText('0 / 0')).toBeInTheDocument()
  })

  it('shows a load error with a retry that loads the numbers', async () => {
    let failing = true
    const { user } = renderApp('/', {
      me: ADMIN_ME,
      api: {
        [`GET ${OVERVIEW_PATH}`]: () =>
          failing ? { status: 500, body: errorEnvelope('internal') } : { body: overviewView() },
      },
    })

    const alert = await screen.findByRole('alert')
    expect(within(alert).getByText('无法加载概览')).toBeInTheDocument()
    failing = false
    await user.click(within(alert).getByRole('button', { name: '重试' }))

    expect(await screen.findByText('12 / 40')).toBeInTheDocument()
  })

  it('follows the server: a refetch updates the tiles, a failed one keeps them and warns', async () => {
    let served: ReturnType<typeof overviewView> | null = overviewView()
    const { queryClient } = renderApp('/', {
      me: ADMIN_ME,
      api: {
        [`GET ${OVERVIEW_PATH}`]: () =>
          served === null ? { status: 503, body: errorEnvelope('internal') } : { body: served },
      },
    })
    await screen.findByText('12 / 40')

    served = overviewView({ devices: { online: 20, total: 41 } })
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: getGetOverviewQueryKey() })
    })
    expect(await screen.findByText('20 / 41')).toBeInTheDocument()

    served = null
    await act(async () => {
      await queryClient.refetchQueries({ queryKey: getGetOverviewQueryKey() })
    })
    expect(await screen.findByText('无法刷新概览', { exact: false })).toBeInTheDocument()
    expect(screen.getByText('20 / 41')).toBeInTheDocument()
  })

  it('does not ask a plain user for the overview', async () => {
    const { fetchSpy } = renderApp('/', {
      me: USER_ME,
      api: { [`GET ${OVERVIEW_PATH}`]: { body: overviewView() } },
    })

    await screen.findByRole('heading', { level: 1 })
    expect(callsTo(fetchSpy, OVERVIEW_PATH)).toBe(0)
  })
})
