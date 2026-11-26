import { fireEvent, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import { pageOf } from '@/features/users/test-data'

import { alarmView, ALARMS_PATH, CONNECTIONS_PATH, connectionView } from '../test-data'

const BOB_CONNECTION = connectionView(1)
const HOSTILE = connectionView(3, { peer_name: '<img src=x onerror=alert(1)>' })

describe('audit page: connections', () => {
  it('renders device-supplied text as text, never as markup', async () => {
    const { container } = renderApp('/audit?item=3', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([HOSTILE]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(document.body.querySelector('img')).toBeNull()
    expect(container.querySelector('[onerror]')).toBeNull()
  })

  it('drops the end of the window when the start is moved past it, and the reverse', async () => {
    const start = new Date(2026, 2, 10).getTime()
    const end = new Date(2026, 2, 13).getTime()
    const { router } = renderApp(`/audit?from=${String(start)}&to=${String(end)}`, {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]) },
    })
    await screen.findByText('Reception PC')

    fireEvent.change(screen.getByLabelText('起始日期'), { target: { value: '2026-03-20' } })
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ from: new Date(2026, 2, 20).getTime() })
    })

    fireEvent.change(screen.getByLabelText('截止日期'), { target: { value: '2026-03-05' } })
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ to: new Date(2026, 2, 6).getTime() })
    })
  })

  it('isolates device-supplied text from the text around it', async () => {
    renderApp('/audit?item=1', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('Reception PC').closest('bdi')).not.toBeNull()
    expect(
      within(drawer).getByRole('heading', { level: 2 }).querySelector('bdi'),
    ).toHaveTextContent('100200300')
    const table = screen.getByRole('table', { name: '连接', hidden: true })
    expect(within(table).getByText('Reception PC').closest('bdi')).not.toBeNull()
  })
})

describe('audit page: tabs', () => {
  it('shows alarm details as text, never as markup', async () => {
    renderApp('/audit?tab=alarms&item=1', {
      api: { [`GET ${ALARMS_PATH}`]: pageOf([alarmView(1)]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('<b>mallory</b>')).toBeInTheDocument()
    expect(drawer.querySelector('b')).toBeNull()
  })
})

describe('audit page: access', () => {
  it('keeps plain users out and asks the API nothing', async () => {
    const { fetchSpy } = renderApp('/audit', { me: USER_ME })

    expect(await screen.findByRole('heading', { name: '无权访问' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, CONNECTIONS_PATH)).toBe(0)
  })
})
