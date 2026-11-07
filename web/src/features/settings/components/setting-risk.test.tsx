// M4-09: `auth.oidc.link_email_max_role` warns while it lets the identity provider reach
// administrator or owner accounts by email.

import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { SettingView } from '@/api/generated/model'
import { renderApp } from '@/test/render-app'

import { sampleSettings, SETTINGS_PATH } from '../test-data'

const KEY = 'auth.oidc.link_email_max_role'
const WARNING = /身份提供方可以凭邮箱接管管理员或所有者账号/

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
    schema: { type: 'enum', variants: ['off', 'user', 'admin', 'owner'] },
  }
  return [...sampleSettings(), ceiling]
}

describe('the email link role ceiling', () => {
  it('warns while admin or owner is chosen, and not for user', async () => {
    const { user } = renderApp('/settings', {
      api: { [`GET ${SETTINGS_PATH}`]: { body: withCeiling('user') } },
    })
    const control = await screen.findByLabelText(KEY)

    expect(screen.queryByText(WARNING)).not.toBeInTheDocument()
    await user.selectOptions(control, 'admin')
    expect(screen.getByText(WARNING)).toBeInTheDocument()
    await user.selectOptions(control, 'off')
    expect(screen.queryByText(WARNING)).not.toBeInTheDocument()
  })

  it('warns for a stored owner ceiling', async () => {
    renderApp('/settings', {
      api: { [`GET ${SETTINGS_PATH}`]: { body: withCeiling('owner') } },
    })

    expect(await screen.findByText(WARNING)).toBeInTheDocument()
  })
})
