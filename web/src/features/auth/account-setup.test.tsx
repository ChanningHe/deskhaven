import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ApiError } from '@/api/api-error'
import type { Me } from '@/api/generated/model'
import { OWNER_ME, meFor } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

const PASSWORD_STEP: Me = { ...meFor('owner'), setup_required: 'password_change' }
const TOTP_STEP: Me = { ...meFor('owner'), totp_enabled: false, setup_required: 'totp_enrolment' }

const ENROLMENT = {
  secret: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP',
  uri: 'otpauth://totp/deskhaven:owner?secret=JBSWY3DPEHPK3PXP',
  qr: { size: 33, path: 'M4 4h7v1h-7z' },
}

describe('account setup', () => {
  it('keeps a restricted session out of the console and on its step', async () => {
    const { router } = renderApp('/devices', { me: TOTP_STEP })

    expect(await screen.findByRole('heading', { name: '设置两步验证' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/account/two-factor')
    expect(screen.queryByRole('navigation')).not.toBeInTheDocument()
  })

  it('moves to the setup page when the server starts answering setup_required', async () => {
    let calls = 0
    const { queryClient, router } = renderApp('/devices', {
      api: {
        'GET /admin/api/v1/me': () => {
          calls += 1
          return { body: calls === 1 ? OWNER_ME : TOTP_STEP }
        },
      },
    })
    await screen.findByRole('heading', { level: 1, name: '设备' })

    await expect(
      queryClient.query({
        queryKey: ['devices'],
        queryFn: () => Promise.reject(new ApiError(403, { code: 'setup_required' })),
        retry: false,
      }),
    ).rejects.toThrow()

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/account/two-factor')
    })
  })

  it('sends a session without a pending step away from the setup pages', async () => {
    const { router } = renderApp('/account/password', { me: OWNER_ME })

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })

  it('changes a temporary password, keeping wrong current passwords in place', async () => {
    let attempt = 0
    const { user, router } = renderApp('/account/password', {
      me: PASSWORD_STEP,
      api: {
        'POST /admin/api/v1/me/password': () => {
          attempt += 1
          return attempt === 1
            ? { status: 422, body: errorEnvelope('wrong_password') }
            : { body: OWNER_ME }
        },
      },
    })

    const fill = async () => {
      await user.type(await screen.findByLabelText('当前密码'), 'temporary')
      await user.type(screen.getByLabelText('新密码'), 'brand new password')
      await user.type(screen.getByLabelText('确认新密码'), 'brand new password')
      await user.click(screen.getByRole('button', { name: '修改密码' }))
    }
    await fill()
    expect(await screen.findByText('当前密码错误。')).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/account/password')
    await user.clear(screen.getByLabelText('当前密码'))
    await user.clear(screen.getByLabelText('新密码'))
    await user.clear(screen.getByLabelText('确认新密码'))
    await fill()

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })

  it('enrols TOTP: QR and key, the current password and a code, then the recovery codes once', async () => {
    let sent: unknown
    let attempt = 0
    const enabled = {
      recovery_codes: ['aaaaa-bbbbb', 'ccccc-ddddd'],
      user: { ...OWNER_ME, totp_enabled: true },
    }
    const { user, router, fetchSpy } = renderApp('/account/two-factor', {
      me: TOTP_STEP,
      api: {
        'POST /admin/api/v1/me/totp/enrolment': { body: ENROLMENT },
        'POST /admin/api/v1/me/totp/enrolment/confirm': ({ init }) => {
          attempt += 1
          sent = JSON.parse(typeof init.body === 'string' ? init.body : 'null')
          return attempt === 1
            ? { status: 422, body: errorEnvelope('wrong_password') }
            : { body: enabled }
        },
      },
    })

    const qr = await screen.findByRole('img', { name: '供验证器应用扫描的二维码' })
    expect(qr.getAttribute('viewBox')).toBe('0 0 33 33')
    expect(qr.querySelector('path')?.getAttribute('d')).toBe('M4 4h7v1h-7z')
    expect(screen.getByText('JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP')).toBeInTheDocument()
    expect(callsTo(fetchSpy, '/admin/api/v1/me/totp/enrolment')).toBe(1)

    await user.type(screen.getByLabelText('当前密码'), 'wrong')
    await user.type(screen.getByLabelText('应用中的验证码'), '123456')
    await user.click(screen.getByRole('button', { name: '启用' }))
    expect(await screen.findByText('当前密码错误。')).toBeInTheDocument()
    await user.clear(screen.getByLabelText('当前密码'))
    await user.type(screen.getByLabelText('当前密码'), 'my password')
    await user.type(screen.getByLabelText('应用中的验证码'), '654321')
    await user.click(screen.getByRole('button', { name: '启用' }))
    const dialog = await screen.findByRole('dialog', { name: '恢复码' })
    expect(sent).toEqual({ current_password: 'my password', code: '654321' })
    expect(within(dialog).getByRole('textbox')).toHaveValue('aaaaa-bbbbb\nccccc-ddddd')
    await user.click(within(dialog).getByRole('checkbox'))
    await user.click(within(dialog).getByRole('button', { name: '完成' }))

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })
})
