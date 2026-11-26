import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { USER_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { account, ACCOUNT_PATH, API } from '../test-data'

const PASSWORD_PATH = `${API}/me/password`

function open(overrides: Parameters<typeof account>[0]) {
  return renderApp('/me', {
    me: USER_ME,
    api: { [`GET ${ACCOUNT_PATH}`]: { body: account(overrides) } },
  })
}

describe('My account: password', () => {
  it('changes the password after the two new entries match', async () => {
    let sent: unknown
    const { user: actor } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account() },
        [`POST ${PASSWORD_PATH}`]: ({ init }) => {
          sent = JSON.parse(init.body as string)
          return { body: USER_ME }
        },
      },
    })

    await actor.type(await screen.findByLabelText('当前密码'), 'old secret')
    await actor.type(screen.getByLabelText('新密码'), 'brand new secret')
    await actor.type(screen.getByLabelText('确认新密码'), 'brand new secrex')
    const submit = screen.getByRole('button', { name: '修改密码' })
    expect(submit).toBeDisabled()
    await actor.clear(screen.getByLabelText('确认新密码'))
    await actor.type(screen.getByLabelText('确认新密码'), 'brand new secret')
    await actor.click(submit)

    await waitFor(() => {
      expect(sent).toEqual({ current_password: 'old secret', new_password: 'brand new secret' })
    })
    await waitFor(() => {
      expect(screen.getByLabelText('当前密码')).toHaveValue('')
    })
  })

  it.each(['ldap', 'oidc'] as const)('offers a %s account no password form', async (authSource) => {
    open({ auth_source: authSource, can_change_password: false })

    await screen.findByRole('region', { name: '密码' })
    expect(screen.queryByLabelText('新密码')).not.toBeInTheDocument()
  })
})

describe('My account: two-factor authentication', () => {
  it('enrols with a code and shows the recovery codes once', async () => {
    let confirmed: unknown
    let enabled = false
    const { user: actor, fetchSpy } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: () => ({ body: account({ totp_enabled: enabled }) }),
        [`POST ${API}/me/totp/enrolment`]: {
          body: {
            secret: 'JBSWY3DPEHPK3PXP',
            uri: 'otpauth://totp/x',
            qr: { size: 29, path: 'M4 4h1v1h-1z' },
          },
        },
        [`POST ${API}/me/totp/enrolment/confirm`]: ({ init }) => {
          confirmed = JSON.parse(init.body as string)
          enabled = true
          return {
            body: { recovery_codes: ['aaaa-bbbb', 'cccc-dddd'], user: { ...USER_ME } },
          }
        },
      },
    })

    await actor.click(await screen.findByRole('button', { name: '启用两步验证' }))
    const dialog = await screen.findByRole('dialog', { name: '启用两步验证' })
    expect(await within(dialog).findByText('JBSW Y3DP EHPK 3PXP')).toBeInTheDocument()
    expect(within(dialog).getByRole('img')).toBeInTheDocument()
    await actor.type(within(dialog).getByLabelText('当前密码'), 'my password')
    await actor.type(within(dialog).getByLabelText('应用中的验证码'), '123456')
    await actor.click(within(dialog).getByRole('button', { name: '启用' }))

    const reveal = await screen.findByRole('dialog', { name: '恢复码' })
    expect(confirmed).toEqual({ current_password: 'my password', code: '123456' })
    expect(within(reveal).getByRole('textbox')).toHaveValue('aaaa-bbbb\ncccc-dddd')
    await actor.click(within(reveal).getByRole('checkbox'))
    await actor.click(within(reveal).getByRole('button', { name: '完成' }))

    await waitFor(() => {
      expect(callsTo(fetchSpy, ACCOUNT_PATH)).toBeGreaterThanOrEqual(2)
    })
    expect(screen.queryByText('aaaa-bbbb')).not.toBeInTheDocument()
  })

  it('turns TOTP off after the password, and shows a wrong password in the dialog', async () => {
    let off = false
    const sent: unknown[] = []
    const { user: actor } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: () => ({
          body: account({ totp_enabled: !off, recovery_codes_left: off ? 0 : 8 }),
        }),
        [`POST ${API}/me/totp/disable`]: ({ init }) => {
          sent.push(JSON.parse(init.body as string))
          if (sent.length === 1) return { status: 422, body: errorEnvelope('wrong_password') }
          off = true
          return { body: USER_ME }
        },
      },
    })

    await actor.click(await screen.findByRole('button', { name: '关闭两步验证' }))
    const dialog = await screen.findByRole('dialog', { name: '关闭两步验证' })
    await actor.type(within(dialog).getByLabelText('当前密码'), 'nope')
    await actor.type(within(dialog).getByLabelText('验证码或恢复码'), '123456')
    await actor.click(within(dialog).getByRole('button', { name: '确认关闭' }))
    expect(await within(dialog).findByRole('alert')).toBeInTheDocument()
    // A failed attempt clears both secrets.
    expect(within(dialog).getByLabelText('验证码或恢复码')).toHaveValue('')
    await actor.type(within(dialog).getByLabelText('当前密码'), 'right')
    await actor.type(within(dialog).getByLabelText('验证码或恢复码'), ' abcd-efgh ')
    await actor.click(within(dialog).getByRole('button', { name: '确认关闭' }))

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })
    expect(sent).toEqual([
      { current_password: 'nope', code: '123456' },
      { current_password: 'right', code: 'abcd-efgh' },
    ])
  })

  it('cannot turn TOTP off when the server requires it', async () => {
    open({ totp_enabled: true, recovery_codes_left: 10, tfa_required: true })

    const off = await screen.findByRole('button', { name: '关闭两步验证' })
    expect(off).toBeDisabled()
  })

  it('regenerates the recovery codes after the password and shows them once', async () => {
    let sent: unknown
    const { user: actor } = renderApp('/me', {
      me: USER_ME,
      api: {
        [`GET ${ACCOUNT_PATH}`]: { body: account({ totp_enabled: true, recovery_codes_left: 2 }) },
        [`POST ${API}/me/totp/recovery-codes`]: ({ init }) => {
          sent = JSON.parse(init.body as string)
          return { body: { recovery_codes: ['new1-new1', 'new2-new2'] } }
        },
      },
    })

    await actor.click(await screen.findByRole('button', { name: '重新生成恢复码' }))
    const dialog = await screen.findByRole('dialog', { name: '重新生成恢复码' })
    await actor.type(within(dialog).getByLabelText('当前密码'), 'pw')
    await actor.type(within(dialog).getByLabelText('验证码或恢复码'), '654321')
    await actor.click(within(dialog).getByRole('button', { name: '重新生成' }))

    const reveal = await screen.findByRole('dialog', { name: '恢复码' })
    expect(sent).toEqual({ current_password: 'pw', code: '654321' })
    expect(within(reveal).getByRole('textbox')).toHaveValue('new1-new1\nnew2-new2')
  })

  it('offers single sign-on accounts no local second factor', async () => {
    open({ auth_source: 'oidc', can_change_password: false, can_use_totp: false })

    await screen.findByRole('region', { name: '两步验证' })
    expect(screen.queryByRole('button', { name: '启用两步验证' })).not.toBeInTheDocument()
  })
})
