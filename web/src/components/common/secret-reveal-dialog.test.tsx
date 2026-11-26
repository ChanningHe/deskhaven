import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { SecretRevealDialog } from './secret-reveal-dialog'

const SECRET = 'dh_tok_9f2c1a7e'

function renderSecret() {
  const onDone = vi.fn()
  const view = renderWithProviders(
    <SecretRevealDialog open title="API token created" secret={SECRET} onDone={onDone} />,
  )
  const dialog = screen.getByRole('dialog', { name: 'API token created' })
  return { ...view, dialog, onDone }
}

describe('SecretRevealDialog', () => {
  it('closes only after the user confirms the secret was saved', async () => {
    const { user, dialog, onDone } = renderSecret()
    const done = within(dialog).getByRole('button', { name: '完成' })

    expect(done).toBeDisabled()
    await user.keyboard('{Escape}')
    await user.click(document.body)
    expect(screen.getByRole('dialog')).toBeInTheDocument()

    await user.click(within(dialog).getByRole('checkbox', { name: '我已妥善保存' }))
    await waitFor(() => {
      expect(done).toBeEnabled()
    })
    await user.click(done)

    expect(onDone).toHaveBeenCalledTimes(1)
  })
})
