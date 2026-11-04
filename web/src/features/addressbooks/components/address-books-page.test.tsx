import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { book, BOOKS, items, page } from '../test-data'

const GUID = '0190aaaa-0000-7000-8000-000000000001'

describe('address books page', () => {
  it('lists the shared books with owner and counts', async () => {
    renderApp('/address-books', {
      api: {
        [`GET ${BOOKS}`]: page([
          book(GUID, 'ops', { owner: 'alice', owner_user_id: 5, peer_count: 3, share_count: 2 }),
          book('g2', 'lab'),
        ]),
      },
    })
    const table = await screen.findByRole('table', { name: '共享地址簿' })
    expect(await within(table).findByRole('button', { name: 'ops' })).toBeInTheDocument()
    expect(within(table).getByText('alice')).toBeInTheDocument()
    // Without an owner the administrators manage the book.
    expect(within(table).getByText('管理员')).toBeInTheDocument()
  })

  it('creates a book with an owner and opens it', async () => {
    let posted: unknown
    const created = book(GUID, 'ops')
    const { router, user } = renderApp('/address-books', {
      api: {
        [`GET ${BOOKS}`]: page([]),
        [`POST ${BOOKS}`]: ({ init }) => {
          posted = JSON.parse(init.body as string)
          return { status: 201, body: created }
        },
        [`GET ${BOOKS}/${GUID}`]: { body: created },
        [`GET ${BOOKS}/${GUID}/peers`]: page([]),
        [`GET ${BOOKS}/${GUID}/tags`]: items([]),
        'GET /admin/api/v1/users': page([]),
      },
    })
    await user.click(await screen.findByRole('button', { name: '新建地址簿' }))
    const dialog = await screen.findByRole('dialog', { name: '新建地址簿' })
    await user.type(within(dialog).getByLabelText('名称'), 'ops')
    await user.click(within(dialog).getByRole('button', { name: '创建地址簿' }))

    expect(await screen.findByRole('dialog', { name: 'ops' })).toBeInTheDocument()
    expect(posted).toEqual({ name: 'ops', note: '' })
    expect(router.state.location.search).toEqual({ book: GUID })
  })

  it('deletes a book only after its name is typed', async () => {
    const ops = book(GUID, 'ops', { peer_count: 2 })
    const { user, fetchSpy } = renderApp(`/address-books?book=${GUID}&tab=details`, {
      api: {
        [`GET ${BOOKS}`]: page([ops]),
        [`GET ${BOOKS}/${GUID}`]: { body: ops },
        [`DELETE ${BOOKS}/${GUID}`]: { status: 204 },
      },
    })
    const drawer = await screen.findByRole('dialog', { name: 'ops' })
    await user.click(await within(drawer).findByRole('button', { name: '删除地址簿' }))

    const confirm = await screen.findByRole('alertdialog', { name: '删除此地址簿？' })
    const submit = within(confirm).getByRole('button', { name: '删除地址簿' })
    expect(submit).toBeDisabled()
    await user.type(within(confirm).getByRole('textbox'), 'ops')
    await user.click(submit)

    await waitFor(() => {
      expect(callsTo(fetchSpy, `${BOOKS}/${GUID}`)).toBeGreaterThan(1)
    })
    expect(fetchSpy.mock.calls.some(([, init]) => init?.method === 'DELETE')).toBe(true)
  })
})
