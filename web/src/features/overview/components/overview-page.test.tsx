import { screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME, USER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { OVERVIEW_PATH, overviewView } from '../test-data'

async function openOverview(view = overviewView()) {
  const app = renderApp('/', { me: ADMIN_ME, api: { [`GET ${OVERVIEW_PATH}`]: { body: view } } })
  await screen.findByText('在线设备')
  return app
}

describe('overview page', () => {
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

  it('shows alarm device text as text, isolated in bdi', async () => {
    await openOverview()

    const alarms = screen.getByRole('region', { name: '最近告警' })
    const items = within(alarms).getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(within(items[0] ?? alarms).getByText('100200300').tagName).toBe('BDI')
    expect(within(items[0] ?? alarms).getByText(/name=<b>mallory<\/b>/).tagName).toBe('BDI')
    expect(alarms.querySelector('b')).toBeNull()
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
