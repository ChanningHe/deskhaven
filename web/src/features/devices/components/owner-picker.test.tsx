import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { pageOf, userView } from '@/features/users/test-data'
import { renderApp } from '@/test/render-app'

import { ALPHA_PATH, drawerApi } from '../test-data'

const USERS = '/admin/api/v1/users'

function app() {
  return renderApp('/devices?device=1', {
    api: drawerApi({ [`GET ${USERS}`]: pageOf([userView(6, 'carol')]) }),
  })
}

describe('owner picker in the device drawer', () => {
  it('keeps the drawer and its edits open on the Escape that clears the search', async () => {
    const { user, router } = app()
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.type(within(drawer).getByLabelText('备注'), ' more')
    await user.click(within(drawer).getByRole('button', { name: '移除所有者' }))
    await user.type(within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' }), 'ca')
    await within(drawer).findByRole('option', { name: /carol/ })

    await user.keyboard('{Escape}')

    expect(within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' })).toHaveValue('')
    expect(screen.getByRole('dialog', { name: 'alpha' })).toBeInTheDocument()
    expect(within(drawer).getByLabelText('备注')).toHaveValue('front desk more')
    expect(router.state.location.search).toEqual({ device: 1 })

    // Nothing left to clear: now Escape closes the drawer as usual.
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
  })

  it('never saves the drawer on Enter in the search field', async () => {
    let patched = 0
    const { user } = renderApp('/devices?device=1', {
      api: drawerApi({
        [`GET ${USERS}`]: pageOf([userView(6, 'carol')]),
        [`PATCH ${ALPHA_PATH}`]: () => {
          patched += 1
          return { body: { status: 200 } }
        },
      }),
    })
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })
    await user.type(within(drawer).getByLabelText('备注'), '!')
    await user.click(within(drawer).getByRole('button', { name: '移除所有者' }))
    await user.type(
      within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' }),
      'zz{Enter}',
    )
    expect(patched).toBe(0)
  })

  it('moves focus to the search after removing the owner and to the remove button after picking', async () => {
    const { user } = app()
    const drawer = await screen.findByRole('dialog', { name: 'alpha' })

    await user.click(within(drawer).getByRole('button', { name: '移除所有者' }))
    const search = within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' })
    await waitFor(() => {
      expect(search).toHaveFocus()
    })

    await user.type(search, 'ca')
    await user.click(await within(drawer).findByRole('option', { name: /carol/ }))
    const remove = await within(drawer).findByRole('button', { name: '移除所有者' })
    await waitFor(() => {
      expect(remove).toHaveFocus()
    })
    expect(within(drawer).getByText('carol')).toBeInTheDocument()
  })
})
