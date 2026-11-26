import { screen, waitFor, within } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import { renderWithProviders } from '@/test/render-with-providers'

import { ConfirmDialog } from './confirm-dialog'
import type { ConfirmDialogProps } from './confirm-dialog'

type HarnessProps = Pick<ConfirmDialogProps, 'onConfirm'> & { confirmText?: string }

function Harness({ onConfirm, confirmText }: HarnessProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        onClick={() => {
          setOpen(true)
        }}
      >
        Delete device
      </button>
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title="Delete device office-pc?"
        description="The device is removed from every group and address book."
        confirmLabel="Delete"
        onConfirm={onConfirm}
        {...(confirmText === undefined ? {} : { confirmText })}
      />
    </>
  )
}

async function openDialog(props: HarnessProps) {
  const view = renderWithProviders(<Harness {...props} />)
  await view.user.click(screen.getByRole('button', { name: 'Delete device' }))
  const dialog = await screen.findByRole('alertdialog', { name: 'Delete device office-pc?' })
  return { ...view, dialog }
}

describe('ConfirmDialog', () => {
  it('keeps the action disabled until the exact name is typed', async () => {
    const onConfirm = vi.fn()
    const { user, dialog } = await openDialog({ onConfirm, confirmText: 'office-pc' })

    const input = within(dialog).getByRole('textbox')
    const confirm = within(dialog).getByRole('button', { name: 'Delete' })
    expect(confirm).toBeDisabled()

    await user.type(input, 'office-p')
    expect(confirm).toBeDisabled()
    await user.type(input, 'C')
    expect(confirm).toBeDisabled()
    await user.clear(input)
    await user.type(input, 'office-pc')
    expect(confirm).toBeEnabled()

    await user.click(confirm)

    expect(onConfirm).toHaveBeenCalledTimes(1)
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
  })

  it('confirms with Enter once the name matches', async () => {
    const onConfirm = vi.fn()
    const { user, dialog } = await openDialog({ onConfirm, confirmText: 'office-pc' })
    const input = within(dialog).getByRole('textbox')

    await user.type(input, 'wrong{Enter}')
    expect(onConfirm).not.toHaveBeenCalled()

    await user.clear(input)
    await user.type(input, 'office-pc{Enter}')
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it('starts empty every time it opens', async () => {
    const { user, dialog } = await openDialog({ onConfirm: vi.fn(), confirmText: 'office-pc' })
    await user.type(within(dialog).getByRole('textbox'), 'office-pc')
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })

    await user.click(screen.getByRole('button', { name: 'Delete device' }))

    expect(await screen.findByRole('textbox')).toHaveValue('')
  })

  it('stays open and shows an error when the action fails', async () => {
    const onConfirm = vi.fn(() => Promise.reject(new ApiError(409, { code: 'conflict' })))
    const { user, dialog } = await openDialog({ onConfirm })

    await user.click(within(dialog).getByRole('button', { name: 'Delete' }))

    expect(await within(dialog).findByRole('alert')).toBeInTheDocument()
    expect(screen.getByRole('alertdialog')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeEnabled()
  })

  it('cannot be dismissed while the action runs', async () => {
    let finish: () => void = () => undefined
    const onConfirm = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve
        }),
    )
    const { user, dialog } = await openDialog({ onConfirm })

    await user.click(within(dialog).getByRole('button', { name: 'Delete' }))
    expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeDisabled()
    await user.keyboard('{Escape}')
    expect(screen.getByRole('alertdialog')).toBeInTheDocument()

    finish()
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
  })
})
