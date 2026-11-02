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
  it('shows the secret read-only and warns that it is shown once', () => {
    const { dialog } = renderSecret()

    const field = within(dialog).getByRole('textbox', { name: 'API token created' })
    expect(field).toHaveValue(SECRET)
    expect(field).toHaveAttribute('readonly')
    expect(dialog).toHaveTextContent('此内容只显示这一次')
  })

  it('copies the secret to the clipboard', async () => {
    const { user, dialog } = renderSecret()
    // user-event installs its own clipboard stub; read back through it.
    await user.click(within(dialog).getByRole('button', { name: '复制' }))

    expect(await within(dialog).findByRole('status')).toHaveTextContent('已复制到剪贴板')
    await expect(navigator.clipboard.readText()).resolves.toBe(SECRET)
  })

  it('selects the text for manual copying when the clipboard is unavailable', async () => {
    const { user, dialog } = renderSecret()
    vi.spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'))

    await user.click(within(dialog).getByRole('button', { name: '复制' }))

    expect(await within(dialog).findByRole('status')).toHaveTextContent('无法访问剪贴板')
    const field = within(dialog).getByRole('textbox')
    expect(field).toHaveFocus()
  })

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
