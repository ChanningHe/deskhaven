import { screen, waitFor, within } from '@testing-library/react'
import i18n from 'i18next'
import { describe, expect, it } from 'vitest'

import type { SettingView } from '@/api/generated/model'
import { ADMIN_ME } from '@/test/fixtures'
import { errorEnvelope } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { settingName } from '../labels'
import { sampleSettings, SETTINGS_PATH, withSetting } from '../test-data'

const TTL = 'auth.session_ttl'
const MIN_LENGTH = 'auth.password.min_length'

const field = (key: string) => screen.getByLabelText(settingName(key, i18n.t))

function one(settings: readonly SettingView[], key: string): SettingView {
  const found = settings.find((s) => s.key === key)
  if (found === undefined) throw new Error(`no setting ${key}`)
  return found
}

function modified(value: unknown, version = 3): Partial<SettingView> {
  return {
    value,
    version,
    source: { kind: 'database', version, by: 7, by_username: 'alice', at: 1_700_000_000_000 },
  }
}

/** A stateful stub of the settings endpoints: GET lists, PATCH answers with `onPatch`. */
function settingsApi(initial: SettingView[], onPatch: (body: unknown) => ReturnType<MockHandler>) {
  const state = { settings: initial, patches: [] as unknown[] }
  const api = {
    [`GET ${SETTINGS_PATH}`]: () => ({ body: state.settings }),
    [`PATCH ${SETTINGS_PATH}`]: ({ init }: { init: RequestInit }) => {
      const body: unknown = JSON.parse(init.body as string)
      state.patches.push(body)
      return onPatch(body)
    },
  }
  return { state, api }
}

async function openPage(
  initial: SettingView[],
  onPatch = (): ReturnType<MockHandler> => ({ body: initial }),
) {
  const stub = settingsApi(initial, onPatch)
  const app = renderApp('/settings', { api: stub.api })
  await screen.findByLabelText(settingName(TTL, i18n.t))
  return { ...app, ...stub }
}

describe('settings page', () => {
  it('saves a change with the version it was read at', async () => {
    const initial = sampleSettings()
    const after = withSetting(initial, MIN_LENGTH, modified(12, 1))
    const { user, state } = await openPage(initial, () => ({ body: after }))

    const input = field(MIN_LENGTH)
    await user.clear(input)
    await user.type(input, '12')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(state.patches).toEqual([{ changes: [{ key: MIN_LENGTH, version: 0, value: 12 }] }])
    })
    expect(field(MIN_LENGTH)).toHaveValue('12')
  })

  it('keeps a relay list typed line by line and saves every line', async () => {
    const RELAYS = 'signal.relay_servers'
    const initial = withSetting(sampleSettings(), RELAYS, { value: ['a:1'] })
    const { user, state } = await openPage(initial)

    const input = field(RELAYS)
    await user.type(input, '{Enter}')
    expect(input).toHaveValue('a:1\n')
    await user.type(input, 'b:2')
    expect(input).toHaveValue('a:1\nb:2')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(state.patches).toEqual([
        { changes: [{ key: RELAYS, version: 0, value: ['a:1', 'b:2'] }] },
      ])
    })
  })

  it('shows settings pinned by a variable or a file read-only', async () => {
    const pinned = withSetting(
      withSetting(sampleSettings(), TTL, {
        value: '7d',
        pinned: true,
        source: { kind: 'env', var: 'DESKHAVEN_SETTINGS__AUTH__SESSION_TTL' },
      }),
      'auth.registration',
      {
        value: 'invite',
        pinned: true,
        source: { kind: 'file', path: '/etc/deskhaven/deskhaven.toml' },
      },
    )
    await openPage(pinned)

    expect(field(TTL)).toBeDisabled()
    expect(field(TTL)).toHaveValue('7d')
    expect(field('auth.registration')).toBeDisabled()
    const row = field(TTL).closest('li') as HTMLElement
    expect(within(row).queryByRole('button', { name: '恢复默认' })).not.toBeInTheDocument()
  })

  it('refuses an invalid value before it is sent', async () => {
    const { user, state } = await openPage(sampleSettings())

    const input = field(MIN_LENGTH)
    await user.clear(input)
    await user.type(input, '3')

    expect(screen.getByRole('button', { name: '保存更改' })).toBeDisabled()
    expect(state.patches).toEqual([])
  })

  it('keeps a secret write-only: never shown, sent only when typed', async () => {
    const initial = sampleSettings()
    const after = withSetting(initial, 'smtp.password', modified({ set: true }, 1))
    const { user, state } = await openPage(initial, () => ({ body: after }))

    const input = field('smtp.password')
    expect(input).toHaveValue('')
    await user.type(input, 's3cret-value')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(field('smtp.password')).toHaveValue('')
    })
    expect(state.patches).toEqual([
      { changes: [{ key: 'smtp.password', version: 0, value: 's3cret-value' }] },
    ])
    expect(document.body.textContent).not.toContain('s3cret-value')
  })

  it('stages a reset to the default and sends it on save', async () => {
    const initial = withSetting(sampleSettings(), TTL, modified('60d', 2))
    const after = withSetting(initial, TTL, {
      value: '30d',
      version: 0,
      source: { kind: 'default' },
    })
    const { user, state } = await openPage(initial, () => ({ body: after }))

    const row = field(TTL).closest('li') as HTMLElement
    await user.click(within(row).getByRole('button', { name: '恢复默认' }))
    expect(field(TTL)).toBeDisabled()
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(state.patches).toEqual([{ changes: [{ key: TTL, version: 2, reset: true }] }])
    })
    await waitFor(() => {
      expect(field(TTL)).toHaveValue('30d')
    })
  })

  it('shows the server reasons of a rejected batch under the key', async () => {
    const { user } = await openPage(sampleSettings(), () => ({
      status: 422,
      body: {
        error: {
          code: 'validation_failed',
          message: 'invalid input',
          details: { fields: { [TTL]: ['must be between 5m and 365d'] } },
        },
      },
    }))

    const input = field(TTL)
    await user.clear(input)
    await user.type(input, '1m')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    expect(await screen.findByText(/must be between 5m and 365d/)).toBeInTheDocument()
    expect(field(TTL)).toHaveValue('1m')
  })
})

describe('settings page conflicts', () => {
  const CONFLICT = (current: SettingView): ReturnType<MockHandler> => ({
    status: 409,
    body: {
      error: {
        code: 'version_conflict',
        message: 'settings changed since they were read',
        details: { keys: [current.key], conflicts: [current] },
      },
    },
  })

  it('shows my value next to theirs and lets me keep mine on top of theirs', async () => {
    const theirs = {
      ...withSetting(sampleSettings(), TTL, modified('90d', 5))[0],
    } as SettingView
    const saved = withSetting(sampleSettings(), TTL, modified('60d', 6))
    let calls = 0
    const { user, state } = await openPage(sampleSettings(), () => {
      calls += 1
      return calls === 1 ? CONFLICT(theirs) : { body: saved }
    })

    const input = field(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    const entry = within(dialog).getByText(TTL).closest('li') as HTMLElement
    expect(within(entry).getByText('60d')).toBeInTheDocument()
    expect(within(entry).getByText('90d')).toBeInTheDocument()
    expect(within(entry).getByText(/alice/)).toBeInTheDocument()

    await user.click(within(dialog).getByRole('button', { name: '保留我的值' }))

    await waitFor(() => {
      expect(state.patches).toEqual([
        { changes: [{ key: TTL, version: 0, value: '60d' }] },
        { changes: [{ key: TTL, version: 5, value: '60d' }] },
      ])
    })
  })

  it('takes their value and drops my edit when I choose to reload', async () => {
    const theirsList = withSetting(sampleSettings(), TTL, modified('90d', 5))
    const theirs = one(theirsList, TTL)
    const { user, state } = await openPage(sampleSettings(), () => CONFLICT(theirs))
    // The reload shows the server's list from here on.
    state.settings = theirsList

    const input = field(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await user.click(within(dialog).getByRole('button', { name: '采用对方的值' }))

    await waitFor(() => {
      expect(field(TTL)).toHaveValue('90d')
    })
    expect(state.patches).toHaveLength(1)
  })

  it('keeps editing after closing the dialog', async () => {
    const theirs = one(withSetting(sampleSettings(), TTL, modified('90d', 5)), TTL)
    const { user } = await openPage(sampleSettings(), () => CONFLICT(theirs))

    const input = field(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await user.click(within(dialog).getByRole('button', { name: '继续编辑' }))

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
    expect(field(TTL)).toHaveValue('60d')
  })

  it('shows a secret conflict without revealing either side', async () => {
    const theirs = {
      ...one(sampleSettings(), 'smtp.password'),
      ...modified({ set: true }, 2),
    }
    const { user } = await openPage(sampleSettings(), () => CONFLICT(theirs))

    await user.type(field('smtp.password'), 'my-new-secret')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    expect(dialog.textContent).not.toContain('my-new-secret')
  })
})

describe('settings page failed saves', () => {
  const SECRET_TEXT = 'my-new-secret'

  function cachedMutations(client: ReturnType<typeof renderApp>['queryClient']): string {
    return JSON.stringify(
      client
        .getMutationCache()
        .getAll()
        .map((m) => m.state.variables),
    )
  }

  it('shows the failure and keeps no request with a plaintext secret around', async () => {
    const { user, queryClient } = await openPage(sampleSettings(), () => ({
      status: 500,
      body: errorEnvelope('internal'),
    }))

    await user.type(field('smtp.password'), SECRET_TEXT)
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(field('smtp.password')).toHaveValue(SECRET_TEXT)
    await waitFor(() => {
      expect(cachedMutations(queryClient)).not.toContain(SECRET_TEXT)
    })
  })

  it('keeps no secret request after a conflict either', async () => {
    const theirs = one(sampleSettings(), 'smtp.password')
    const { user, queryClient } = await openPage(sampleSettings(), () => ({
      status: 409,
      body: {
        error: {
          code: 'version_conflict',
          message: 'conflict',
          details: { keys: [theirs.key], conflicts: [{ ...theirs, version: 2 }] },
        },
      },
    }))

    await user.type(field('smtp.password'), SECRET_TEXT)
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await waitFor(() => {
      expect(cachedMutations(queryClient)).not.toContain(SECRET_TEXT)
    })
  })

  it('drops edits of a key that became pinned and keeps the others', async () => {
    const pinned = withSetting(sampleSettings(), TTL, {
      value: '7d',
      pinned: true,
      source: { kind: 'env', var: 'DESKHAVEN_SETTINGS__AUTH__SESSION_TTL' },
    })
    let refreshed = false
    const { user, state } = await openPage(sampleSettings(), () => {
      // The server pinned the key meanwhile: the next list shows it.
      refreshed = true
      return {
        status: 409,
        body: {
          error: { code: 'setting_pinned', message: 'pinned', details: { keys: [TTL] } },
        },
      }
    })

    const input = field(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.selectOptions(field('auth.registration'), 'open')
    state.settings = pinned
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(field(TTL)).toBeDisabled()
    })
    expect(refreshed).toBe(true)
    expect(field(TTL)).toHaveValue('7d')
    expect(field('auth.registration')).toHaveValue('open')
  })
})

describe('settings page history and guards', () => {
  it('asks before leaving the page with unsaved changes', async () => {
    const { user, router } = await openPage(sampleSettings())

    await user.selectOptions(field('auth.registration'), 'open')
    void router.navigate({ to: '/groups' })

    const dialog = await screen.findByRole('alertdialog', { name: '放弃未保存的修改？' })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))
    expect(router.state.location.pathname).toBe('/settings')
    expect(field('auth.registration')).toHaveValue('open')

    void router.navigate({ to: '/groups' })
    const again = await screen.findByRole('alertdialog', { name: '放弃未保存的修改？' })
    await user.click(within(again).getByRole('button', { name: '放弃修改' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/groups')
    })
  })

  it('is for owners only', async () => {
    renderApp('/settings', {
      me: ADMIN_ME,
      api: {
        [`GET ${SETTINGS_PATH}`]: { status: 403, body: errorEnvelope('forbidden') },
      },
    })

    await waitFor(() => {
      expect(screen.queryByText(TTL)).not.toBeInTheDocument()
    })
    expect(screen.queryByRole('heading', { name: '认证与会话' })).not.toBeInTheDocument()
  })
})
