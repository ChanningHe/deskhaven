import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { DeviceView } from '@/api/generated/model'
import { callsTo } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { book, BOOKS, items, page, peer, share, tag } from '../test-data'

const GUID = '0190aaaa-0000-7000-8000-000000000001'
const BOOK = `${BOOKS}/${GUID}`
const SECRET = 'correct-horse-battery'

function body(init: RequestInit): unknown {
  return JSON.parse(init.body as string)
}

function device(id: number, rustdeskId: string, hostname: string): DeviceView {
  return {
    id,
    rustdesk_id: rustdeskId,
    hostname,
    username: 'alice',
    os: 'linux',
    version: '1.4.1',
    cpu: null,
    memory: null,
    note: '',
    online: false,
    status: 'active',
    owner: null,
    device_group: null,
    last_ip: '192.0.2.7',
    last_seen_at: 1_700_000_000_000,
    first_seen_at: 1_700_000_000_000,
    created_at: 1_700_000_000_000,
  }
}

/** Routes every drawer test needs: the book itself and (empty) lists for the rest. */
function drawerApi(extra: Record<string, MockHandler | { status?: number; body?: unknown }> = {}) {
  const ops = book(GUID, 'ops', { peer_count: 1 })
  return {
    [`GET ${BOOKS}`]: page([ops]),
    [`GET ${BOOK}`]: { body: ops },
    [`GET ${BOOK}/peers`]: page([]),
    [`GET ${BOOK}/tags`]: items([]),
    [`GET ${BOOK}/shares`]: items([]),
    ...extra,
  }
}

describe('address book entries', () => {
  it('shows that an entry has a password and never the password itself', async () => {
    const stored = peer('100', { alias: 'Front desk', has_password: true, tags: ['lobby'] })
    renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({ [`GET ${BOOK}/peers`]: page([stored]) }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })

    expect(await within(drawer).findByText('Front desk')).toBeInTheDocument()
    expect(within(drawer).getByText('已设密码')).toBeInTheDocument()
    expect(within(drawer).getByText('lobby')).toBeInTheDocument()
    expect(drawer.textContent).not.toContain(SECRET)
  })

  it('adds an entry and sends the password once, in the request only', async () => {
    let posted: unknown
    const { user } = renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({
        [`POST ${BOOK}/peers`]: ({ init }) => {
          posted = body(init)
          return { status: 201, body: peer('300', { has_password: true }) }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('button', { name: '添加条目' }))
    const form = await screen.findByRole('dialog', { name: '添加条目' })
    await user.type(within(form).getByLabelText('RustDesk ID'), '300')
    await user.type(within(form).getByLabelText('别名'), 'Lab PC')
    const password = within(form).getByLabelText('密码')
    expect(password).toHaveAttribute('type', 'password')
    await user.type(password, SECRET)
    await user.click(within(form).getByRole('button', { name: '添加条目' }))

    await waitFor(() => {
      expect(posted).toMatchObject({ id: '300', alias: 'Lab PC', password: SECRET })
    })
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: '添加条目' })).not.toBeInTheDocument()
    })
    expect(document.body.textContent).not.toContain(SECRET)
  })

  it('edits only the changed fields and keeps the stored password unless replaced', async () => {
    let patched: unknown
    const stored = peer('100', { alias: 'Front desk', hostname: 'pc-1', has_password: true })
    const { user } = renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({
        [`GET ${BOOK}/peers`]: page([stored]),
        [`PATCH ${BOOK}/peers/100`]: ({ init }) => {
          patched = body(init)
          return { body: stored }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('button', { name: '100' }))
    const form = await screen.findByRole('dialog', { name: '条目 100' })
    const password = within(form).getByLabelText('密码')
    expect(password).toHaveValue('')
    expect(password).toHaveAttribute('placeholder', '不输入则保留已存的密码')
    const alias = within(form).getByLabelText('别名')
    await user.clear(alias)
    await user.type(alias, 'Lobby')
    await user.click(within(form).getByRole('button', { name: '保存条目' }))

    await waitFor(() => {
      expect(patched).toEqual({ alias: 'Lobby' })
    })
  })

  it('removes a stored password with an explicit choice', async () => {
    let patched: unknown
    const stored = peer('100', { has_password: true })
    const { user } = renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({
        [`GET ${BOOK}/peers`]: page([stored]),
        [`PATCH ${BOOK}/peers/100`]: ({ init }) => {
          patched = body(init)
          return { body: peer('100') }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('button', { name: '100' }))
    const form = await screen.findByRole('dialog', { name: '条目 100' })
    await user.click(within(form).getByRole('checkbox', { name: '删除已存的密码' }))
    await user.click(within(form).getByRole('button', { name: '保存条目' }))

    await waitFor(() => {
      expect(patched).toEqual({ password: '' })
    })
  })

  it('asks before removing the selected entries', async () => {
    let removed: unknown
    const { user, fetchSpy } = renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({
        [`GET ${BOOK}/peers`]: page([peer('100'), peer('200')]),
        [`POST ${BOOK}/peers/delete`]: ({ init }) => {
          removed = body(init)
          return { status: 204 }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('checkbox', { name: '选择 200' }))
    await user.click(within(drawer).getByRole('button', { name: '移除所选' }))

    const confirm = await screen.findByRole('alertdialog', { name: '移除这些条目？' })
    expect(callsTo(fetchSpy, `${BOOK}/peers/delete`)).toBe(0)
    await user.click(within(confirm).getByRole('button', { name: '移除' }))
    await waitFor(() => {
      expect(removed).toEqual({ ids: ['200'] })
    })
  })
})

describe('importing devices', () => {
  it('adds the picked devices and reports what happened', async () => {
    let posted: unknown
    const { user } = renderApp(`/address-books?book=${GUID}`, {
      api: drawerApi({
        'GET /admin/api/v1/devices': page([device(7, '100', 'DESKTOP-1'), device(8, '200', 'box')]),
        [`POST ${BOOK}/peers/import`]: ({ init }) => {
          posted = body(init)
          return { body: { added: ['100'], existing: ['200'], missing: [] } }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('button', { name: '导入设备' }))
    const dialog = await screen.findByRole('dialog', { name: '导入设备' })
    await user.click(await within(dialog).findByRole('checkbox', { name: /100/ }))
    await user.click(within(dialog).getByRole('checkbox', { name: /200/ }))
    await user.click(within(dialog).getByRole('button', { name: '添加所选 2 台' }))

    await waitFor(() => {
      expect(posted).toEqual({ device_ids: [7, 8] })
    })
    expect(await within(dialog).findByRole('status')).toHaveTextContent(
      '已添加 1 台，已在列表中 1 台，未找到 0 台。',
    )
  })
})

describe('address book tags', () => {
  it('adds a tag with an opaque ARGB colour', async () => {
    let posted: unknown
    const { user } = renderApp(`/address-books?book=${GUID}&tab=tags`, {
      api: drawerApi({
        [`GET ${BOOK}/tags`]: items([tag('lobby')]),
        [`POST ${BOOK}/tags`]: ({ init }) => {
          posted = body(init)
          return { status: 201, body: tag('db') }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    expect(await within(drawer).findByText('lobby')).toBeInTheDocument()
    await user.type(within(drawer).getByLabelText('标签名'), 'db')
    await user.click(within(drawer).getByRole('button', { name: '添加标签' }))

    await waitFor(() => {
      expect(posted).toEqual({ name: 'db', color: 0xff9e9e9e })
    })
  })
})

describe('address book grants', () => {
  it('warns that any grant hands out the stored passwords', async () => {
    renderApp(`/address-books?book=${GUID}&tab=shares`, { api: drawerApi() })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })

    const note = await within(drawer).findByRole('note')
    expect(note).toHaveTextContent('包括只读')
    expect(note).toHaveTextContent('明文')
  })

  it('lists grants, changes a rule, grants to a group and revokes after confirming', async () => {
    const puts: unknown[] = []
    let revoked = false
    const { user, fetchSpy } = renderApp(`/address-books?book=${GUID}&tab=shares`, {
      api: drawerApi({
        [`GET ${BOOK}/shares`]: items([share('user', 5, 'alice', 1)]),
        [`PUT ${BOOK}/shares`]: ({ init }) => {
          puts.push(body(init))
          return { status: 204 }
        },
        [`DELETE ${BOOK}/shares/user/5`]: () => {
          revoked = true
          return { status: 204 }
        },
        'GET /admin/api/v1/groups': page([{ id: 9, name: 'team' }]),
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    const rule = await within(drawer).findByRole('combobox', { name: 'alice 的权限' })
    expect(rule).toHaveValue('1')
    await user.selectOptions(rule, '2')
    await waitFor(() => {
      expect(puts).toEqual([{ subject_type: 'user', subject_id: 5, rule: 2 }])
    })

    await user.selectOptions(within(drawer).getByLabelText('授权给'), 'group')
    await user.selectOptions(await within(drawer).findByLabelText('用户组'), 'team')
    await user.click(within(drawer).getByRole('button', { name: '授权' }))
    await waitFor(() => {
      expect(puts[1]).toEqual({ subject_type: 'group', subject_id: 9, rule: 1 })
    })

    await user.click(within(drawer).getByRole('button', { name: '撤销 alice 的授权' }))
    const confirm = await screen.findByRole('alertdialog', { name: '撤销此授权？' })
    expect(revoked).toBe(false)
    await user.click(within(confirm).getByRole('button', { name: '撤销' }))
    await waitFor(() => {
      expect(revoked).toBe(true)
    })
    expect(callsTo(fetchSpy, `${BOOK}/shares`)).toBeGreaterThan(1)
  })
})
