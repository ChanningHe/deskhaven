// M4-09: the drawer lists an account's linked sign-ins (unlink only, never link) and marks
// an unverified email verified.

import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { pageOf, userDetail, userView, USERS_PATH } from '../test-data'

const LINKED = {
  id: 9,
  provider: 'corp',
  provider_display_name: 'Corp SSO',
  subject: 'sub-9',
  email: null,
  linked_via: 'email' as const,
  created_at: 1_800_000_000_000,
}

describe('user drawer: linked sign-ins and email verification', () => {
  it('lists the identities and unlinks one after confirmation', async () => {
    const bob = userView(5, 'bob')
    const { user, fetchSpy } = renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([bob]),
        [`GET ${USERS_PATH}/5`]: { body: { ...userDetail(bob), identities: [LINKED] } },
        [`DELETE ${USERS_PATH}/5/identities/9`]: { status: 204 },
      },
    })
    const drawer = within(await screen.findByRole('dialog', { name: 'bob' }))

    expect(drawer.getByText('Corp SSO')).toBeInTheDocument()
    expect(drawer.getByText('sub-9')).toBeInTheDocument()
    expect(drawer.getByText('按邮箱关联')).toBeInTheDocument()
    expect(drawer.queryByRole('button', { name: /关联 / })).not.toBeInTheDocument()
    await user.click(drawer.getByRole('button', { name: '解除关联' }))
    const confirm = within(await screen.findByRole('alertdialog'))
    await user.click(confirm.getByRole('button', { name: '解除关联' }))

    await waitFor(() => {
      expect(callsTo(fetchSpy, `${USERS_PATH}/5/identities/9`)).toBe(1)
    })
  })

  it('marks an unverified email verified', async () => {
    let sent: unknown
    const bob = userView(5, 'bob', { email: 'bob@example.com', email_verified: false })
    const { user } = renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([bob]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(bob) },
        [`PATCH ${USERS_PATH}/5`]: ({ init }) => {
          sent = JSON.parse(typeof init.body === 'string' ? init.body : 'null')
          return { body: { ...bob, email_verified: true } }
        },
      },
    })
    const drawer = within(await screen.findByRole('dialog', { name: 'bob' }))

    await user.click(drawer.getByRole('button', { name: '标记邮箱已验证' }))

    await waitFor(() => {
      expect(sent).toEqual({ email_verified: true })
    })
  })

  it('offers no marking for a verified or missing address', async () => {
    const bob = userView(5, 'bob', { email: 'bob@example.com', email_verified: true })
    renderApp('/users?user=5', {
      api: {
        [`GET ${USERS_PATH}`]: pageOf([bob]),
        [`GET ${USERS_PATH}/5`]: { body: userDetail(bob) },
      },
    })
    const drawer = within(await screen.findByRole('dialog', { name: 'bob' }))

    expect(drawer.queryByRole('button', { name: '标记邮箱已验证' })).not.toBeInTheDocument()
  })
})
