import { screen } from '@testing-library/react'
import i18n from 'i18next'
import { describe, expect, it } from 'vitest'

import type { SettingView } from '@/api/generated/model'
import { renderApp } from '@/test/render-app'

import { settingName } from '../labels'
import { sampleSettings, SETTINGS_PATH } from '../test-data'

const KEY = 'auth.oidc.link_email_max_role'

function withCeiling(value: string): SettingView[] {
  const ceiling: SettingView = {
    key: KEY,
    category: 'auth',
    description: 'Highest role reached by email linking.',
    value,
    default: 'user',
    source: { kind: 'default' },
    pinned: false,
    sensitive: false,
    version: 0,
    schema: {
      type: 'enum',
      variants: ['off', 'user', 'admin', 'owner'],
      env: 'DESKHAVEN_SETTINGS__AUTH__OIDC__LINK_EMAIL_MAX_ROLE',
      pin_only: false,
    },
  }
  return [...sampleSettings(), ceiling]
}

describe('the email link role ceiling', () => {
  it('warns while admin or owner is chosen, and not for user', async () => {
    const { user } = renderApp('/settings', {
      api: { [`GET ${SETTINGS_PATH}`]: { body: withCeiling('user') } },
    })
    const control = await screen.findByLabelText(settingName(KEY, i18n.t))

    expect(screen.queryByRole('note')).not.toBeInTheDocument()
    await user.selectOptions(control, 'admin')
    expect(screen.getByRole('note')).toBeInTheDocument()
    await user.selectOptions(control, 'off')
    expect(screen.queryByRole('note')).not.toBeInTheDocument()
  })

  it('warns for a stored owner ceiling', async () => {
    renderApp('/settings', {
      api: { [`GET ${SETTINGS_PATH}`]: { body: withCeiling('owner') } },
    })

    expect(await screen.findByRole('note')).toBeInTheDocument()
  })
})
