import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import type { RenderAppOptions } from '@/test/render-app'
import { callsTo, requestedUrl } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { account, ACCOUNT_PATH, SESSIONS_PATH, session, sessionsPage } from '../test-data'

const CURRENT = session(11, { current: true, user_agent: 'Firefox 140' })
const OTHER = session(12, { user_agent: 'Safari 19', ip: '198.51.100.4' })
const CLIENT = session(13, {
  kind: 'client',
  user_agent: null,
  rustdesk_id: '123456789',
  device_name: 'work laptop',
  device_os: 'linux',
})

function open(extra: NonNullable<RenderAppOptions['api']> = {}) {
  return renderApp('/me', {
    me: USER_ME,
    api: {
      [`GET ${ACCOUNT_PATH}`]: { body: account() },
      [`GET ${SESSIONS_PATH}`]: { body: sessionsPage([CURRENT, OTHER, CLIENT]) },
      ...extra,
    },
  })
}

async function openSessionsTab(actor: ReturnType<typeof renderApp>['user'], waitFor: string) {
  await actor.click(await screen.findByRole('tab', { name: '会话' }))
  await screen.findByText(waitFor)
}

function rowOf(text: string): HTMLElement {
  const row = screen.getByText(text).closest('tr')
  if (row === null) throw new Error(`no row for ${text}`)
  return row
}

describe('My account: sessions', () => {
  it('lists client and console sessions, marks the current one, isolates device text', async () => {
    const { user: actor } = open()

    await openSessionsTab(actor, 'Firefox 140')

    expect(within(rowOf('Firefox 140')).getByText('当前会话')).toBeInTheDocument()
    expect(within(rowOf('work laptop')).getByText('客户端')).toBeInTheDocument()
    expect(screen.getByText('work laptop').closest('bdi')).not.toBeNull()
    expect(screen.getByText('Safari 19').closest('bdi')).not.toBeNull()
    expect(within(rowOf('Safari 19')).queryByText('当前会话')).not.toBeInTheDocument()
  })

  it('asks for the largest page and says when older sessions are not shown', async () => {
    const { user: actor, fetchSpy } = open({
      [`GET ${SESSIONS_PATH}`]: { body: sessionsPage([CURRENT, OTHER, CLIENT], 250) },
    })

    await openSessionsTab(actor, 'Firefox 140')

    expect(screen.getByText('仅显示最近使用的 3 / 250 个会话。')).toBeInTheDocument()
    const urls = fetchSpy.mock.calls.map((_, index) => requestedUrl(fetchSpy, index))
    const listed = urls.find((url) => url.pathname === SESSIONS_PATH)
    expect(listed?.searchParams.get('page_size')).toBe('200')
  })

  it('revokes another session after confirmation and refreshes the list', async () => {
    let revoked = false
    const { user: actor, fetchSpy } = open({
      [`GET ${SESSIONS_PATH}`]: () => ({
        body: sessionsPage(revoked ? [CURRENT, CLIENT] : [CURRENT, OTHER, CLIENT]),
      }),
      [`DELETE ${SESSIONS_PATH}/12`]: () => {
        revoked = true
        return { status: 204 }
      },
    })
    await openSessionsTab(actor, 'Safari 19')

    await actor.click(within(rowOf('Safari 19')).getByRole('button', { name: /^吊销会话/ }))
    const dialog = await screen.findByRole('alertdialog')
    await actor.click(within(dialog).getByRole('button', { name: '吊销' }))

    await waitFor(() => {
      expect(screen.queryByText('Safari 19')).not.toBeInTheDocument()
    })
    expect(callsTo(fetchSpy, `${SESSIONS_PATH}/12`)).toBe(1)
  })

  it('revokes all other sessions in one step', async () => {
    let calls = 0
    const { user: actor } = open({
      [`POST ${SESSIONS_PATH}/revoke-others`]: () => {
        calls += 1
        return { body: { revoked: 2 } }
      },
    })
    await openSessionsTab(actor, 'Firefox 140')

    await actor.click(screen.getByRole('button', { name: '吊销其他会话' }))
    const dialog = await screen.findByRole('alertdialog')
    await actor.click(within(dialog).getByRole('button', { name: '吊销其他会话' }))

    await waitFor(() => {
      expect(calls).toBe(1)
    })
  })

  it('leaves for the login page after revoking the current session', async () => {
    const { user: actor, router } = open({ [`DELETE ${SESSIONS_PATH}/11`]: { status: 204 } })
    await openSessionsTab(actor, 'Firefox 140')

    await actor.click(within(rowOf('Firefox 140')).getByRole('button', { name: /^吊销会话/ }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText(/退出登录/)).toBeInTheDocument()
    await actor.click(within(dialog).getByRole('button', { name: '吊销' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/login')
    })
  })
})
