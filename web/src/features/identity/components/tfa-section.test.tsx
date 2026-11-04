import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ADMIN_ME } from '@/test/fixtures'
import { callsTo, errorEnvelope } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { DIRECTORIES_PATH, PROVIDERS_PATH, SETTINGS_PATH, tfaSetting } from '../test-data'

const BASE = {
  [`GET ${PROVIDERS_PATH}`]: { body: { callback_url: null, items: [] } },
  [`GET ${DIRECTORIES_PATH}`]: { body: [] },
}

function tfaSection() {
  return within(screen.getByRole('region', { name: '双因素认证' }))
}

describe('identity page: 2FA policy', () => {
  it('saves the owner’s choice with the version it was read at', async () => {
    let patched: unknown
    const { user } = renderApp('/identity', {
      api: {
        ...BASE,
        [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting()] },
        [`PATCH ${SETTINGS_PATH}`]: ({ init }) => {
          patched = JSON.parse(init.body as string)
          return {
            body: [
              tfaSetting({
                value: 'admins',
                version: 4,
                source: { kind: 'database', at: 1, version: 4 },
              }),
            ],
          }
        },
      },
    })
    const select = await screen.findByRole('combobox', { name: '强制双因素认证的范围' })
    const save = tfaSection().getByRole('button', { name: '保存策略' })
    expect(save).toBeDisabled()

    await user.selectOptions(select, 'admins')
    await user.click(save)

    await waitFor(() => {
      expect(tfaSection().getByText('策略已保存。')).toBeInTheDocument()
    })
    expect(patched).toEqual({
      changes: [{ key: 'auth.tfa.enforce', value: 'admins', version: 3 }],
    })
    expect(select).toHaveValue('admins')
  })

  it('is read-only while an environment variable pins it', async () => {
    renderApp('/identity', {
      api: {
        ...BASE,
        [`GET ${SETTINGS_PATH}`]: {
          body: [
            tfaSetting({
              value: 'all',
              pinned: true,
              source: { kind: 'env', var: 'DESKHAVEN_AUTH_TFA_ENFORCE' },
            }),
          ],
        },
      },
    })

    const select = await screen.findByRole('combobox', { name: '强制双因素认证的范围' })
    expect(select).toBeDisabled()
    expect(select).toHaveValue('all')
    expect(tfaSection().getByText(/DESKHAVEN_AUTH_TFA_ENFORCE/)).toBeInTheDocument()
    expect(tfaSection().queryByRole('button', { name: '保存策略' })).not.toBeInTheDocument()
  })

  it('tells about a concurrent change', async () => {
    const { user } = renderApp('/identity', {
      api: {
        ...BASE,
        [`GET ${SETTINGS_PATH}`]: { body: [tfaSetting()] },
        [`PATCH ${SETTINGS_PATH}`]: { status: 409, body: errorEnvelope('version_conflict') },
      },
    })
    const select = await screen.findByRole('combobox', { name: '强制双因素认证的范围' })
    await user.selectOptions(select, 'all')
    await user.click(tfaSection().getByRole('button', { name: '保存策略' }))

    expect(await tfaSection().findByText(/他人修改了此策略/)).toBeInTheDocument()
  })

  it('shows admins that only the owner decides, without reading the settings', async () => {
    const { fetchSpy } = renderApp('/identity', { me: ADMIN_ME, api: BASE })

    expect(await screen.findByText('只有所有者可以修改此策略。')).toBeInTheDocument()
    expect(callsTo(fetchSpy, SETTINGS_PATH)).toBe(0)
  })
})
