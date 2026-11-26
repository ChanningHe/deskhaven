import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { pageOf, userView } from '@/features/users/test-data'
import { renderApp } from '@/test/render-app'

import { drawerApi } from '../test-data'

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
    await screen.findByRole('option', { name: /carol/ })

    await user.keyboard('{Escape}')

    expect(within(drawer).getByRole('combobox', { name: '搜索用户作为所有者' })).toHaveValue('')
    expect(screen.getByRole('dialog', { name: 'alpha' })).toBeInTheDocument()
    expect(within(drawer).getByLabelText('备注')).toHaveValue('front desk more')
    expect(router.state.location.search).toEqual({ device: 1 })

    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({})
    })
  })
})
