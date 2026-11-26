import { screen } from '@testing-library/react'
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

describe('user combobox', () => {
  it('ignores Enter while the suggestions still answer older text', async () => {
    const { user, picked, input } = setup()
    await user.click(input)
    await screen.findByRole('option', { name: /carol/ })
    await user.type(input, 'd{Enter}')
    expect(input).toHaveAttribute('aria-busy', 'true')
    expect(picked).not.toHaveBeenCalled()
  })

  it('never submits the surrounding form on Enter while there is text', async () => {
    const { user, picked, submitted, input } = setup()
    await user.type(input, 'zz{Enter}')
    expect(submitted).not.toHaveBeenCalled()
    expect(picked).not.toHaveBeenCalled()
    expect(input).toHaveValue('zz')
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
})
