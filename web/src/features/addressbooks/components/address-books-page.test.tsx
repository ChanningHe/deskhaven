import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { book, BOOKS, page } from '../test-data'

const GUID = '0190aaaa-0000-7000-8000-000000000001'

describe('address books page', () => {
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
