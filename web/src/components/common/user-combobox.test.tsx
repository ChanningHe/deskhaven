import { act, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { UserView } from '@/api/generated/model'
import { pageOf, userView } from '@/features/users/test-data'
import { mockApi } from '@/test/mock-api'
import { renderWithProviders } from '@/test/render-with-providers'

import { UserCombobox } from './user-combobox'
import type { UserComboboxProps } from './user-combobox'

const USERS = '/admin/api/v1/users'

function setup(
  onPick: (user: UserView) => boolean | undefined = () => undefined,
  extra: Partial<UserComboboxProps> = {},
) {
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
        {...extra}
      />
      <button type="button">Next</button>
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

/** The input names this option as the active one (the highlighted suggestion). */
function expectActive(input: HTMLElement, name: RegExp) {
  const option = screen.getByRole('option', { name })
  expect(option.id).not.toBe('')
  expect(input).toHaveAttribute('aria-activedescendant', option.id)
}

describe('user combobox', () => {
  it('lists the first accounts when the field is clicked, and hides them on blur', async () => {
    const { user, input } = setup()
    await user.click(input)
    const list = await screen.findByRole('listbox', { name: 'Matches' })
    expect(list).toBeVisible()
    expect(screen.getByRole('option', { name: /carol/ })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: /dave/ })).toBeInTheDocument()

    // Base UI closes the popup once focus has settled outside the field.
    await user.tab()
    await waitFor(() => {
      expect(screen.queryByRole('option')).not.toBeInTheDocument()
    })
  })

  it('stays closed on programmatic focus and leaves the page reachable', async () => {
    const { user, input } = setup()
    act(() => {
      input.focus()
    })
    expect(input).toHaveFocus()
    await settled(input)
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('option')).not.toBeInTheDocument()
    // An open list would aria-hide everything around the field.
    expect(screen.getByRole('button', { name: 'Next' })).toBeInTheDocument()

    // ArrowDown is intent: it opens the list.
    await user.keyboard('{ArrowDown}')
    expect(await screen.findByRole('option', { name: /carol/ })).toBeInTheDocument()
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
    expectActive(input, /carol/)

    await user.keyboard('{ArrowDown}')
    expectActive(input, /dave/)
    await user.keyboard('{ArrowDown}')
    expectActive(input, /carol/)
    await user.keyboard('{ArrowUp}')
    expectActive(input, /dave/)

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

  it('keeps the text and leaves the list closed when the pick fails', async () => {
    const { user, picked, input } = setup(() => false)
    await user.type(input, 'ca')
    await screen.findByRole('option', { name: /carol/ })
    await settled(input)
    await user.keyboard('{Enter}')
    await waitFor(() => {
      expect(picked).toHaveBeenCalled()
    })
    expect(input).toHaveValue('ca')
    await waitFor(() => {
      expect(screen.queryByRole('option')).not.toBeInTheDocument()
    })
    expect(input).toHaveAttribute('aria-expanded', 'false')
  })

  it('ignores picks while an earlier one is being handled', async () => {
    const { user, picked, input } = setup(() => undefined, { busy: true })
    await user.click(input)
    const carol = await screen.findByRole('option', { name: /carol/ })
    expect(input).toHaveAttribute('aria-busy', 'true')

    await user.click(carol)
    await user.keyboard('{Enter}')
    expect(picked).not.toHaveBeenCalled()
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

  it('closes an open list on Escape even with an empty field, keeping the key from the document', async () => {
    const { user, input } = setup()
    const seen = vi.fn()
    document.addEventListener('keydown', seen)
    await user.click(input)
    await screen.findByRole('option', { name: /carol/ })
    seen.mockClear()

    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('option')).not.toBeInTheDocument()
    })
    expect(seen).not.toHaveBeenCalled()

    // The list is closed and the field empty: the next Escape is the container's.
    await user.keyboard('{Escape}')
    expect(seen).toHaveBeenCalledTimes(1)
    document.removeEventListener('keydown', seen)
  })
})
