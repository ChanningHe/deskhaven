import { fireEvent, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { callsTo, requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'
import { pageOf } from '@/features/users/test-data'

import {
  alarmView,
  CONNECTIONS_PATH,
  CONSOLE_PATH,
  connectionView,
  consoleView,
  FILES_PATH,
  fileView,
  LOGINS_PATH,
  loginView,
  ALARMS_PATH,
} from '../test-data'

type Spy = Parameters<typeof requestedUrl>[0]

function requestsTo(spy: Spy, pathname: string): URL[] {
  return spy.mock.calls
    .map((_, index) => requestedUrl(spy, index))
    .filter((url) => url.pathname === pathname)
}

const BOB_CONNECTION = connectionView(1)
const RETIRED = connectionView(2, { session_id: null, peer_name: null, controller: null })
const HOSTILE = connectionView(3, { peer_name: '<img src=x onerror=alert(1)>' })

describe('audit page: connections', () => {
  it('lists connections with device, peer, controller, type and state', async () => {
    renderApp('/audit', { api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION, RETIRED]) } })

    await screen.findByText('Reception PC')
    const table = screen.getByRole('table', { name: '连接' })
    const [, first, second] = within(table).getAllByRole('row')
    const row = within(first ?? table)
    expect(row.getByText('100200300')).toBeInTheDocument()
    expect(row.getByText('Reception PC')).toBeInTheDocument()
    expect(row.getByText('bob')).toBeInTheDocument()
    expect(row.getByText('远程控制')).toBeInTheDocument()
    expect(row.getByText('已结束')).toBeInTheDocument()
    expect(within(second ?? table).getByText('900800700')).toBeInTheDocument()
  })

  it('says the data is device self-reported, above the table and in the drawer', async () => {
    const { user } = renderApp('/audit', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]) },
    })
    expect(await screen.findByText(/由设备自行上报/)).toBeInTheDocument()
    await screen.findByText('Reception PC')

    const table = screen.getByRole('table', { name: '连接' })
    await user.click(within(table).getByRole('button', { name: /^\d{4}/ }))

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByRole('heading', { name: '设备自报' })).toBeInTheDocument()
    expect(within(drawer).getByText('Reception PC')).toBeInTheDocument()
    expect(within(drawer).getByText('临时密码')).toBeInTheDocument()
  })

  it('shows a retired row as having no session id, never the stored value', async () => {
    const { user } = renderApp('/audit?item=2', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION, RETIRED]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('无会话 ID')).toBeInTheDocument()
    expect(drawer.textContent).not.toContain('0#')
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
  })

  it('keeps the page when a drawer is opened and closed', async () => {
    const { router, user } = renderApp('/audit?page=2', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION], 50) },
    })
    await screen.findByText('Reception PC')

    await user.click(screen.getByRole('button', { name: /^\d{4}/ }))
    await screen.findByRole('dialog')
    expect(router.state.location.search).toMatchObject({ page: 2, item: 1 })
    await user.keyboard('{Escape}')

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
    expect(router.state.location.search).toMatchObject({ page: 2 })
    expect(router.state.location.search).not.toHaveProperty('item')
  })

  it('renders device-supplied text as text, never as markup', async () => {
    const { container } = renderApp('/audit?item=3', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([HOSTILE]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument()
    expect(document.body.querySelector('img')).toBeNull()
    expect(container.querySelector('[onerror]')).toBeNull()
  })

  it('shows an unknown connection type as its number', async () => {
    renderApp('/audit', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([connectionView(1, { conn_type: 9 })]) },
    })

    expect(await screen.findByText('未知（9）')).toBeInTheDocument()
  })

  it('puts the time window and the filters into the request and the URL', async () => {
    const { router, fetchSpy } = renderApp('/audit', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]) },
    })
    await screen.findByText('Reception PC')

    fireEvent.change(screen.getByLabelText('起始日期'), { target: { value: '2026-03-10' } })
    fireEvent.change(screen.getByLabelText('截止日期'), { target: { value: '2026-03-12' } })
    fireEvent.change(screen.getByRole('combobox', { name: '连接类型' }), { target: { value: '1' } })

    await waitFor(() => {
      expect(router.state.location.search).toMatchObject({
        from: new Date(2026, 2, 10).getTime(),
        to: new Date(2026, 2, 13).getTime(),
        conn_type: 1,
      })
    })
    await waitFor(() => {
      const last = requestsTo(fetchSpy, CONNECTIONS_PATH).at(-1)
      expect(last?.searchParams.get('from')).toBe(String(new Date(2026, 2, 10).getTime()))
      expect(last?.searchParams.get('to')).toBe(String(new Date(2026, 2, 13).getTime()))
      expect(last?.searchParams.get('conn_type')).toBe('1')
    })
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

  it('applies a device id on Enter', async () => {
    const { router, user } = renderApp('/audit', {
      api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]) },
    })
    await screen.findByText('Reception PC')

    await user.type(screen.getByRole('textbox', { name: '设备 ID' }), '100200300{Enter}')

    await waitFor(() => {
      expect(router.state.location.search).toMatchObject({ device: '100200300' })
    })
  })

  it('offers an empty state with no records', async () => {
    renderApp('/audit', { api: { [`GET ${CONNECTIONS_PATH}`]: pageOf([]) } })

    expect(await screen.findByRole('heading', { name: '暂无连接记录' })).toBeInTheDocument()
  })

  it('explains a failed load and retries', async () => {
    const { user, fetchSpy } = renderApp('/audit', {
      api: { [`GET ${CONNECTIONS_PATH}`]: { status: 500, body: { error: { code: 'internal' } } } },
    })

    await user.click(await screen.findByRole('button', { name: /重试/ }))

    await waitFor(() => {
      expect(callsTo(fetchSpy, CONNECTIONS_PATH)).toBeGreaterThan(1)
    })
  })
})

describe('audit page: tabs', () => {
  it('switches to the logins tab, keeping the time window and dropping the rest', async () => {
    const { router, user, fetchSpy } = renderApp('/audit?from=1700000000000&conn_type=1', {
      api: {
        [`GET ${CONNECTIONS_PATH}`]: pageOf([BOB_CONNECTION]),
        [`GET ${LOGINS_PATH}`]: pageOf([
          loginView(1),
          loginView(2, { result: 'invalid_credentials' }),
        ]),
      },
    })
    await screen.findByText('Reception PC')

    await user.click(screen.getByRole('tab', { name: '登录' }))

    const table = await screen.findByRole('table', { name: '登录' })
    expect(await within(table).findByText('凭据无效')).toBeInTheDocument()
    expect(router.state.location.search).toEqual({ tab: 'logins', from: 1_700_000_000_000 })
    const request = requestsTo(fetchSpy, LOGINS_PATH)[0]
    expect(request?.searchParams.get('from')).toBe('1700000000000')
    expect(request?.searchParams.has('conn_type')).toBe(false)
  })

  it('marks the attempted username and client device id of a login as client-supplied', async () => {
    renderApp('/audit?tab=logins', {
      api: { [`GET ${LOGINS_PATH}`]: pageOf([loginView(1)]) },
    })

    expect(
      await screen.findByText(/尝试登录的用户名和客户端设备 ID 由客户端自行提供/),
    ).toBeInTheDocument()
  })

  it('reads the tab and its filters from the URL', async () => {
    const { fetchSpy } = renderApp('/audit?tab=files&direction=send&device=100', {
      api: { [`GET ${FILES_PATH}`]: pageOf([fileView(1)]) },
    })

    await screen.findByText('/home/alice/report.pdf')

    const request = requestsTo(fetchSpy, FILES_PATH)[0]
    expect(request?.searchParams.get('direction')).toBe('send')
    expect(request?.searchParams.get('device')).toBe('100')
  })

  it('shows the transferred files and their sizes in the file drawer', async () => {
    renderApp('/audit?tab=files&item=1', {
      api: { [`GET ${FILES_PATH}`]: pageOf([fileView(1)]) },
    })

    const drawer = await screen.findByRole('dialog')
    const list = within(drawer).getByRole('list', { name: '已传输的文件' })
    expect(within(list).getByText('report.pdf')).toBeInTheDocument()
    expect(within(list).getByText('2 KiB')).toBeInTheDocument()
    expect(within(list).getByText('3 GiB')).toBeInTheDocument()
  })

  it('names alarm types and shows the details as text', async () => {
    renderApp('/audit?tab=alarms&item=1', {
      api: { [`GET ${ALARMS_PATH}`]: pageOf([alarmView(1)]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('尝试超过 30 次')).toBeInTheDocument()
    expect(within(drawer).getByText('<b>mallory</b>')).toBeInTheDocument()
    expect(drawer.querySelector('b')).toBeNull()
  })
})

describe('audit page: console tab', () => {
  const PURGE = consoleView(2, {
    actor: { kind: 'system', id: null, name: null },
    action: 'audit.retention.purge',
    target_type: 'audit',
    target_id: '0198f7a2-0000-7000-8000-000000000001',
    before: null,
    after: { kind: 'conn', deleted: 12, cutoff: 1_700_000_000_000, batch: 500 },
  })

  it('names the actor, falling back to the kind for one that is gone or never existed', async () => {
    renderApp('/audit?tab=console', {
      api: {
        [`GET ${CONSOLE_PATH}`]: pageOf([
          consoleView(1),
          PURGE,
          consoleView(3, { actor: { kind: 'api_token', id: 9, name: null } }),
        ]),
      },
    })

    await screen.findByText('API 令牌 #9')
    const table = screen.getByRole('table', { name: '控制台' })
    expect(within(table).getByText('admin')).toBeInTheDocument()
    expect(within(table).getByText('系统')).toBeInTheDocument()
    expect(within(table).getByText('API 令牌 #9')).toBeInTheDocument()
  })

  it('shows only the changed fields of a change', async () => {
    renderApp('/audit?tab=console&item=1', {
      api: { [`GET ${CONSOLE_PATH}`]: pageOf([consoleView(1)]) },
    })

    const drawer = await screen.findByRole('dialog')
    const changes = within(drawer).getByRole('list', { name: '变更字段' })
    expect(within(changes).getByText('role')).toBeInTheDocument()
    expect(within(changes).getByText('"user"')).toBeInTheDocument()
    expect(within(changes).getByText('"admin"')).toBeInTheDocument()
    expect(within(changes).queryByText('note')).not.toBeInTheDocument()
    expect(within(drawer).getByText('1 个字段未变')).toBeInTheDocument()
  })

  it('summarizes a retention purge run', async () => {
    renderApp('/audit?tab=console&item=2', {
      api: { [`GET ${CONSOLE_PATH}`]: pageOf([consoleView(1), PURGE]) },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText(/删除了 12 条 conn 记录/)).toBeInTheDocument()
    expect(within(drawer).getByText(/批大小 500/)).toBeInTheDocument()
  })

  it('shows masked secrets as stored', async () => {
    renderApp('/audit?tab=console&item=1', {
      api: {
        [`GET ${CONSOLE_PATH}`]: pageOf([
          consoleView(1, {
            before: { password: '<redacted>' },
            after: { password: '<redacted>x' },
          }),
        ]),
      },
    })

    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('"<redacted>"')).toBeInTheDocument()
  })
})

describe('audit page: access', () => {
  it('keeps plain users out and asks the API nothing', async () => {
    const { fetchSpy } = renderApp('/audit', { me: USER_ME })

    expect(await screen.findByRole('heading', { name: '无权访问' })).toBeInTheDocument()
    expect(callsTo(fetchSpy, CONNECTIONS_PATH)).toBe(0)
  })
})
