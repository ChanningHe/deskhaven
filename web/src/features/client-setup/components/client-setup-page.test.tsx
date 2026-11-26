import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

describe('client setup page', () => {
  it('is not for plain users', async () => {
    renderApp('/client-setup', { me: USER_ME })

    await screen.findByText('无权访问')
    expect(screen.queryByRole('heading', { name: '服务器地址' })).not.toBeInTheDocument()
  })
})
