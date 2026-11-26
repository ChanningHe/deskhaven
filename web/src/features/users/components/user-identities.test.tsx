// M4-09: the drawer lists an account's linked sign-ins (unlink only, never link), shows
// whether its email is verified, and marks it verified after a confirmation.

import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { UserDetail } from '@/api/generated/model'
import type { MockHandler, MockResponse } from '@/test/mock-api'
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

/** A drawer whose detail is fetched again after each change (counted). */
function drawer(detail: UserDetail, extra: Record<string, MockResponse | MockHandler> = {}) {
  const counter = { details: 0 }
  const app = renderApp('/users?user=5', {
    api: {
      [`GET ${USERS_PATH}`]: pageOf([detail.user]),
      [`GET ${USERS_PATH}/5`]: () => {
        counter.details += 1
        return { body: detail }
      },
      ...extra,
    },
  })
  return { ...app, counter }
}

async function open() {
  return within(await screen.findByRole('dialog', { name: 'bob' }))
}

describe('user drawer: linked sign-ins', () => {
  it('offers no linking and unlinks one after confirmation', async () => {
    const bob = userView(5, 'bob', { email: 'bob@example.com', email_verified: true })
    const explicit = {
      ...LINKED,
      id: 10,
      subject: 'sub-10',
      provider_display_name: 'Other',
      linked_via: 'explicit' as const,
    }
    const { user, fetchSpy, counter } = drawer(
      { ...userDetail(bob), identities: [LINKED, explicit] },
      { [`DELETE ${USERS_PATH}/5/identities/9`]: { status: 204 } },
    )
    const sheet = await open()

    expect(sheet.queryByRole('button', { name: /^关联 / })).not.toBeInTheDocument()
    await user.click(sheet.getByRole('button', { name: '解除关联 Corp SSO' }))
    const confirm = within(await screen.findByRole('alertdialog'))
    const before = counter.details
    await user.click(confirm.getByRole('button', { name: '解除关联' }))

    await waitFor(() => {
      expect(callsTo(fetchSpy, `${USERS_PATH}/5/identities/9`)).toBe(1)
    })
    await waitFor(() => {
      expect(counter.details).toBeGreaterThan(before)
    })
  })
})

describe('user drawer: email verification', () => {
  it('marks the checked address verified after a confirmation', async () => {
    let sent: unknown
    const bob = userView(5, 'bob', { email: 'bob@example.com', email_verified: false })
    const { user, counter } = drawer(userDetail(bob), {
      [`PATCH ${USERS_PATH}/5`]: ({ init }) => {
        sent = JSON.parse(typeof init.body === 'string' ? init.body : 'null')
        return { body: { ...bob, email_verified: true } }
      },
    })
    const sheet = await open()

    await user.click(sheet.getByRole('button', { name: '标记邮箱已验证' }))
    const confirm = within(await screen.findByRole('alertdialog'))
    const before = counter.details
    await user.click(confirm.getByRole('button', { name: '标记已验证' }))

    await waitFor(() => {
      expect(sent).toEqual({ email_verified: true, expected_email: 'bob@example.com' })
    })
    await waitFor(() => {
      expect(counter.details).toBeGreaterThan(before)
    })
  })

  it('offers no marking for a verified address', async () => {
    const bob = userView(5, 'bob', { email: 'bob@example.com', email_verified: true })
    drawer(userDetail(bob))
    const sheet = await open()

    expect(sheet.queryByRole('button', { name: '标记邮箱已验证' })).not.toBeInTheDocument()
  })
})
