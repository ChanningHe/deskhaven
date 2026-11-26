import { screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { OWNER_ME } from '@/test/fixtures'
import { renderApp } from '@/test/render-app'

const STATUS = 'GET /admin/api/v1/setup'
const SETUP = 'POST /admin/api/v1/setup'

describe('first-run setup', () => {
  it('creates the owner with the link token, then shows the public URL and finishes', async () => {
    let sent: unknown
    const { user, router } = renderApp('/setup#token=tok-123', {
      me: null,
      api: {
        [STATUS]: { body: { required: true, public_url: 'https://rd.example.com' } },
        [SETUP]: ({ init }) => {
          sent = JSON.parse(typeof init.body === 'string' ? init.body : 'null')
          return { body: { status: 'signed_in', user: OWNER_ME } }
        },
      },
    })

    await user.type(await screen.findByLabelText('用户名'), 'root')
    await user.type(screen.getByLabelText('密码'), 'first owner password')
    await user.type(screen.getByLabelText('确认密码'), 'first owner password')
    await waitFor(() => {
      expect(router.state.location.hash).toBe('')
    })
    await user.click(screen.getByRole('button', { name: '创建所有者' }))

    expect(await screen.findByText('https://rd.example.com')).toBeInTheDocument()
    expect(sent).toEqual({
      token: 'tok-123',
      username: 'root',
      display_name: null,
      email: null,
      password: 'first owner password',
    })
    await user.click(screen.getByRole('button', { name: '下一步' }))
    await user.click(await screen.findByRole('button', { name: '进入控制台' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })

  it('refuses mismatched passwords before sending anything', async () => {
    const { user } = renderApp('/setup#token=tok-123', { me: null })

    await user.type(await screen.findByLabelText('密码'), 'one password')
    await user.type(screen.getByLabelText('确认密码'), 'another one')

    expect(screen.getByRole('button', { name: '创建所有者' })).toBeDisabled()
  })
})
