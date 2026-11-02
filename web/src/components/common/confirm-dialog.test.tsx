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
  it('explains the impact and keeps the action disabled until the exact name is typed', async () => {
    const onConfirm = vi.fn()
    const { user, dialog } = await openDialog({ onConfirm, confirmText: 'office-pc' })

    expect(dialog).toHaveAccessibleDescription(
      'The device is removed from every group and address book.',
    )
    const input = within(dialog).getByRole('textbox', { name: '请输入 office-pc 以确认' })
    const confirm = within(dialog).getByRole('button', { name: 'Delete' })
    await waitFor(() => {
      expect(input).toHaveFocus()
    })
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

  it('cancels with Escape or the cancel button without running the action', async () => {
    const onConfirm = vi.fn()
    const { user } = await openDialog({ onConfirm, confirmText: 'office-pc' })

    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
    // Focus returns to the button that opened the dialog.
    expect(screen.getByRole('button', { name: 'Delete device' })).toHaveFocus()

    await user.click(screen.getByRole('button', { name: 'Delete device' }))
    await user.click(await screen.findByRole('button', { name: '取消' }))
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
    expect(onConfirm).not.toHaveBeenCalled()
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

  it('focuses Cancel, not the destructive button, for a plain confirmation', async () => {
    const { dialog } = await openDialog({ onConfirm: vi.fn() })

    expect(within(dialog).queryByRole('textbox')).not.toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Delete' })).toBeEnabled()
    await waitFor(() => {
      expect(within(dialog).getByRole('button', { name: '取消' })).toHaveFocus()
    })
  })

  it('stays open and shows the translated error when the action fails', async () => {
    const onConfirm = vi.fn(() => Promise.reject(new ApiError(409, { code: 'conflict' })))
    const { user, dialog } = await openDialog({ onConfirm })

    await user.click(within(dialog).getByRole('button', { name: 'Delete' }))

    expect(await within(dialog).findByRole('alert')).toHaveTextContent(
      '该对象已被修改或与现有数据冲突，请刷新后重试。',
    )
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
