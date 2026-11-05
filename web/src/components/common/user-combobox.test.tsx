import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { UserView } from '@/api/generated/model'
import { pageOf, userView } from '@/features/users/test-data'
import { mockApi } from '@/test/mock-api'
import { renderWithProviders } from '@/test/render-with-providers'

import { UserCombobox } from './user-combobox'

const USERS = '/admin/api/v1/users'

function setup(onPick: (user: UserView) => boolean | undefined = () => undefined) {
  mockApi({ [`GET ${USERS}`]: pageOf([userView(6, 'carol'), userView(8, 'dave')]) })
  const picked = vi.fn(onPick)
  const submitted = vi.fn((event: { preventDefault: () => void }) => {
    event.preventDefault()
  })
  const view = renderWithProviders(
    <form onSubmit={submitted}>
      <UserCombobox
        label="Find a user"
        listLabel="Matches"
        foundText={(count) => `${String(count)} found`}
        noMatchText="none"
        onPick={picked}
      />
    </form>,
  )
  return {
    ...view,
    picked,
    submitted,
    input: screen.getByRole('combobox', { name: 'Find a user' }),
  }
}

/** The suggestions answer the text as typed (the debounced search has landed). */
async function settled(input: HTMLElement) {
  await waitFor(() => {
    expect(input).toHaveAttribute('aria-busy', 'false')
  })
}

describe('user combobox', () => {
  it('lists the first accounts as soon as the field gets focus, and hides them on blur', async () => {
    const { user, input } = setup()
    await user.click(input)
    const list = await screen.findByRole('listbox', { name: 'Matches' })
    expect(list).toBeVisible()
    expect(screen.getByRole('option', { name: /carol/ })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: /dave/ })).toBeInTheDocument()

    await user.tab()
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
  })

  it('ignores Enter while the suggestions still answer older text', async () => {
    const { user, picked, input } = setup()
    await user.click(input)
    await screen.findByRole('option', { name: /carol/ })
    await user.type(input, 'd{Enter}')
    expect(input).toHaveAttribute('aria-busy', 'true')
    expect(picked).not.toHaveBeenCalled()
  })

  it('moves through the suggestions with the arrow keys, wrapping, and picks with Enter', async () => {
    const { user, picked, submitted, input } = setup()
    await user.type(input, 'a')
    const list = await screen.findByRole('listbox', { name: 'Matches' })
    expect(list).toBeVisible()
    await settled(input)
    expect(screen.getByRole('option', { name: /carol/ })).toHaveAttribute('aria-selected', 'true')

    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('option', { name: /dave/ })).toHaveAttribute('aria-selected', 'true')
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('option', { name: /carol/ })).toHaveAttribute('aria-selected', 'true')
    await user.keyboard('{ArrowUp}')
    expect(screen.getByRole('option', { name: /dave/ })).toHaveAttribute('aria-selected', 'true')

    await user.keyboard('{Enter}')
    await waitFor(() => {
      expect(picked).toHaveBeenCalledWith(expect.objectContaining({ id: 8, username: 'dave' }))
    })
    expect(input).toHaveValue('')
    expect(submitted).not.toHaveBeenCalled()
  })

  it('never submits the surrounding form on Enter while there is text', async () => {
    const { user, picked, submitted, input } = setup()
    await user.type(input, 'zz{Enter}')
    expect(submitted).not.toHaveBeenCalled()
    expect(picked).not.toHaveBeenCalled()
    expect(input).toHaveValue('zz')
  })

  it('keeps the text when the pick fails', async () => {
    const { user, picked, input } = setup(() => false)
    await user.type(input, 'ca')
    await screen.findByRole('option', { name: /carol/ })
    await settled(input)
    await user.keyboard('{Enter}')
    await waitFor(() => {
      expect(picked).toHaveBeenCalled()
    })
    expect(input).toHaveValue('ca')
  })

  it('clears the text on Escape without letting the key reach dismiss handlers around it', async () => {
    const { user, input } = setup()
    const seen = vi.fn()
    document.addEventListener('keydown', seen)
    await user.type(input, 'ca')
    await screen.findByRole('option', { name: /carol/ })
    seen.mockClear()

    await user.keyboard('{Escape}')
    expect(input).toHaveValue('')
    expect(seen).not.toHaveBeenCalled()

    // Nothing to clear: Escape belongs to whatever contains the field.
    await user.keyboard('{Escape}')
    expect(seen).toHaveBeenCalledTimes(1)
    document.removeEventListener('keydown', seen)
  })
})
