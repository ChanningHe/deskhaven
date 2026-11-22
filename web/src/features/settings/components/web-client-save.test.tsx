import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { Me, SettingView } from '@/api/generated/model'
import { ME_QUERY_KEY } from '@/features/auth/session'
import { OWNER_ME } from '@/test/fixtures'
import { callsTo } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { sampleSettings, SETTINGS_PATH, withSetting } from '../test-data'

const ME_PATH = '/admin/api/v1/me'
const ENABLED = 'web_client.enabled'
const MIN_LENGTH = 'auth.password.min_length'

function withWebClient(): SettingView[] {
  const settings = sampleSettings()
  const bool = settings.find((s) => s.key === 'devices.verify_uuid')
  if (bool === undefined) throw new Error('no bool setting')
  return [...settings, { ...bool, key: ENABLED, category: 'web_client', value: false }]
}

/** The settings page on a server where saving `web_client.enabled` turns the remote on. */
function open() {
  const initial = withWebClient()
  const server = { me: OWNER_ME }
  const app = renderApp('/settings', {
    api: {
      [`GET ${ME_PATH}`]: () => ({ body: server.me }),
      [`GET ${SETTINGS_PATH}`]: { body: initial },
      [`PATCH ${SETTINGS_PATH}`]: ({ init }) => {
        const { changes } = JSON.parse(init.body as string) as { changes: { key: string }[] }
        if (changes.some((c) => c.key === ENABLED)) {
          server.me = { ...server.me, web_client_enabled: true }
        }
        return { body: withSetting(initial, ENABLED, { value: true }) }
      },
    },
  })
  return app
}

describe('settings page: browser remote', () => {
  it('fetches the signed-in user again once web_client settings are saved', async () => {
    const { user, queryClient, fetchSpy } = open()
    const input = await screen.findByLabelText(MIN_LENGTH)
    await user.clear(input)
    await user.type(input, '12')
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    await screen.findByText('已保存 1 项设置。')
    expect(callsTo(fetchSpy, ME_PATH)).toBe(1)

    const row = screen.getByText(ENABLED).closest('li') as HTMLElement
    await user.click(within(row).getByRole('checkbox'))
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    await waitFor(() => {
      expect(queryClient.getQueryData<Me>(ME_QUERY_KEY)?.web_client_enabled).toBe(true)
    })
    expect(callsTo(fetchSpy, ME_PATH)).toBe(2)
  })
})
