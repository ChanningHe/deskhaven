import i18n from 'i18next'
import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { Dialog, DialogContent, DialogTitle } from './dialog'

function renderDialog(closeLabel?: string) {
  return renderWithProviders(
    <Dialog open>
      <DialogContent {...(closeLabel === undefined ? {} : { closeLabel })}>
        <DialogTitle>Details</DialogTitle>
      </DialogContent>
    </Dialog>,
  )
}

describe('DialogContent close button', () => {
  it('is named in the UI language by default', async () => {
    renderDialog()
    expect(screen.getByRole('button', { name: '关闭' })).toBeInTheDocument()

    await i18n.changeLanguage('en')
    expect(await screen.findByRole('button', { name: 'Close' })).toBeInTheDocument()
  })

  it('takes an explicit label', () => {
    renderDialog('Dismiss details')
    expect(screen.getByRole('button', { name: 'Dismiss details' })).toBeInTheDocument()
  })
})
