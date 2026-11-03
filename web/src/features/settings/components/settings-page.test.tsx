import { screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { SettingView } from '@/api/generated/model'
import { ADMIN_ME } from '@/test/fixtures'
import { errorEnvelope } from '@/test/mock-api'
import type { MockHandler } from '@/test/mock-api'
import { renderApp } from '@/test/render-app'

import { sampleSettings, SETTINGS_PATH, withSetting } from '../test-data'

const TTL = 'auth.session_ttl'
const MIN_LENGTH = 'auth.password.min_length'

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
  await screen.findByText(TTL)
  return { ...app, ...stub }
}

describe('settings page', () => {
  it('groups settings by category and picks a control per schema type', async () => {
    await openPage(sampleSettings())

    for (const heading of ['认证与会话', '邮件（SMTP）', '设备', '信令']) {
      expect(screen.getByRole('heading', { name: heading })).toBeInTheDocument()
    }
    expect(screen.getByLabelText(TTL)).toHaveValue('30d')
    expect(screen.getByLabelText(MIN_LENGTH)).toHaveValue('10')
    expect(screen.getByLabelText('auth.registration')).toHaveValue('disabled')
    expect(within(screen.getByLabelText('auth.registration')).getAllByRole('option')).toHaveLength(
      3,
    )
    expect(screen.getByLabelText('smtp.password')).toHaveAttribute('type', 'password')
    expect(screen.getByLabelText('signal.relay_servers').tagName).toBe('TEXTAREA')
    expect(screen.getByRole('checkbox', { name: /devices.verify_uuid|启用/ })).toBeChecked()
    expect(screen.getByText('范围 8 至 256。')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /^默认/ })).toHaveLength(sampleSettings().length)
  })

  it('saves a change with the version it was read at and shows the new source', async () => {
    const initial = sampleSettings()
    const after = withSetting(initial, MIN_LENGTH, modified(12, 1))
    const { user, state } = await openPage(initial, () => ({ body: after }))

    const input = screen.getByLabelText(MIN_LENGTH)
    await user.clear(input)
    await user.type(input, '12')
    expect(screen.getByText('1 项未保存的更改')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(screen.getByText('已保存 1 项设置。')).toBeInTheDocument()
    })
    expect(state.patches).toEqual([{ changes: [{ key: MIN_LENGTH, version: 0, value: 12 }] }])
    const row = screen.getByLabelText(MIN_LENGTH).closest('li')
    expect(row).not.toBeNull()
    expect(within(row as HTMLElement).getByRole('button', { name: /^已修改/ })).toBeInTheDocument()
    expect(screen.getByLabelText(MIN_LENGTH)).toHaveValue('12')
  })

  it('shows pinned settings read-only with the variable or file that fixes them', async () => {
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

    expect(screen.getByLabelText(TTL)).toBeDisabled()
    expect(screen.getByLabelText(TTL)).toHaveValue('7d')
    expect(
      screen.getByText('由环境变量 DESKHAVEN_SETTINGS__AUTH__SESSION_TTL 固定，此处只读。'),
    ).toBeInTheDocument()
    expect(screen.getByLabelText('auth.registration')).toBeDisabled()
    expect(
      screen.getByText('由配置文件 /etc/deskhaven/deskhaven.toml 固定，此处只读。'),
    ).toBeInTheDocument()
    const row = screen.getByLabelText(TTL).closest('li') as HTMLElement
    expect(within(row).queryByRole('button', { name: '恢复默认' })).not.toBeInTheDocument()
    expect(within(row).getByRole('button', { name: /^配置锁定/ })).toBeInTheDocument()
  })

  it('refuses an invalid value before it is sent', async () => {
    const { user, state } = await openPage(sampleSettings())

    const input = screen.getByLabelText(MIN_LENGTH)
    await user.clear(input)
    await user.type(input, '3')

    expect(screen.getByText('必须在 8 到 256 之间。')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '保存更改' })).toBeDisabled()
    expect(state.patches).toEqual([])
  })

  it('keeps a secret write-only: never shown, sent only when typed', async () => {
    const initial = sampleSettings()
    const after = withSetting(initial, 'smtp.password', modified({ set: true }, 1))
    const { user, state } = await openPage(initial, () => ({ body: after }))

    const input = screen.getByLabelText('smtp.password')
    expect(input).toHaveValue('')
    await user.type(input, 's3cret-value')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(screen.getByText('已保存 1 项设置。')).toBeInTheDocument()
    })
    expect(state.patches).toEqual([
      { changes: [{ key: 'smtp.password', version: 0, value: 's3cret-value' }] },
    ])
    expect(screen.getByLabelText('smtp.password')).toHaveValue('')
    expect(screen.getByLabelText('smtp.password')).toHaveAttribute(
      'placeholder',
      '已设置。输入新值可替换。',
    )
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

    const row = screen.getByLabelText(TTL).closest('li') as HTMLElement
    await user.click(within(row).getByRole('button', { name: '恢复默认' }))
    expect(screen.getByText('保存后将恢复为默认值。')).toBeInTheDocument()
    expect(screen.getByLabelText(TTL)).toBeDisabled()
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(screen.getByText('已保存 1 项设置。')).toBeInTheDocument()
    })
    expect(state.patches).toEqual([{ changes: [{ key: TTL, version: 2, reset: true }] }])
    expect(screen.getByLabelText(TTL)).toHaveValue('30d')
  })

  it('undoes and discards edits', async () => {
    const { user, state } = await openPage(sampleSettings())

    await user.selectOptions(screen.getByLabelText('auth.registration'), 'open')
    expect(screen.getByText('未保存')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '撤销' }))
    expect(screen.getByLabelText('auth.registration')).toHaveValue('disabled')
    expect(screen.getByText('没有未保存的更改。')).toBeInTheDocument()

    await user.selectOptions(screen.getByLabelText('auth.registration'), 'invite')
    await user.click(screen.getByRole('button', { name: '放弃更改' }))
    expect(screen.getByLabelText('auth.registration')).toHaveValue('disabled')
    expect(state.patches).toEqual([])
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

    const input = screen.getByLabelText(TTL)
    await user.clear(input)
    await user.type(input, '1m')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    expect(
      await screen.findByText('服务器拒绝了该值：must be between 5m and 365d'),
    ).toBeInTheDocument()
    expect(screen.getByLabelText(TTL)).toHaveValue('1m')
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

    const input = screen.getByLabelText(TTL)
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
      expect(screen.getByText('已保存 1 项设置。')).toBeInTheDocument()
    })
    expect(state.patches).toEqual([
      { changes: [{ key: TTL, version: 0, value: '60d' }] },
      { changes: [{ key: TTL, version: 5, value: '60d' }] },
    ])
  })

  it('takes their value and drops my edit when I choose to reload', async () => {
    const theirsList = withSetting(sampleSettings(), TTL, modified('90d', 5))
    const theirs = one(theirsList, TTL)
    const { user, state } = await openPage(sampleSettings(), () => CONFLICT(theirs))
    // The reload shows the server's list from here on.
    state.settings = theirsList

    const input = screen.getByLabelText(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await user.click(within(dialog).getByRole('button', { name: '采用对方的值' }))

    await waitFor(() => {
      expect(screen.getByLabelText(TTL)).toHaveValue('90d')
    })
    expect(screen.getByText('没有未保存的更改。')).toBeInTheDocument()
    expect(state.patches).toHaveLength(1)
  })

  it('keeps editing after closing the dialog', async () => {
    const theirs = one(withSetting(sampleSettings(), TTL, modified('90d', 5)), TTL)
    const { user } = await openPage(sampleSettings(), () => CONFLICT(theirs))

    const input = screen.getByLabelText(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.click(screen.getByRole('button', { name: '保存更改' }))
    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await user.click(within(dialog).getByRole('button', { name: '继续编辑' }))

    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    })
    expect(screen.getByLabelText(TTL)).toHaveValue('60d')
    expect(screen.getByText('1 项未保存的更改')).toBeInTheDocument()
  })

  it('shows a secret conflict without revealing either side', async () => {
    const theirs = {
      ...one(sampleSettings(), 'smtp.password'),
      ...modified({ set: true }, 2),
    }
    const { user } = await openPage(sampleSettings(), () => CONFLICT(theirs))

    await user.type(screen.getByLabelText('smtp.password'), 'my-new-secret')
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    const dialog = await screen.findByRole('alertdialog', { name: '已被他人修改' })
    expect(within(dialog).getByText('新的密钥')).toBeInTheDocument()
    expect(within(dialog).getByText('已设置')).toBeInTheDocument()
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

    await user.type(screen.getByLabelText('smtp.password'), SECRET_TEXT)
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    expect(await screen.findByText('无法保存设置')).toBeInTheDocument()
    expect(screen.getByLabelText('smtp.password')).toHaveValue(SECRET_TEXT)
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

    await user.type(screen.getByLabelText('smtp.password'), SECRET_TEXT)
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await screen.findByRole('alertdialog', { name: '已被他人修改' })
    await waitFor(() => {
      expect(cachedMutations(queryClient)).not.toContain(SECRET_TEXT)
    })
  })

  it('drops edits of a key that became pinned and says so', async () => {
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

    const input = screen.getByLabelText(TTL)
    await user.clear(input)
    await user.type(input, '60d')
    await user.selectOptions(screen.getByLabelText('auth.registration'), 'open')
    state.settings = pinned
    await user.click(screen.getByRole('button', { name: '保存更改' }))

    await waitFor(() => {
      expect(screen.getByLabelText(TTL)).toBeDisabled()
    })
    expect(refreshed).toBe(true)
    expect(screen.getByLabelText(TTL)).toHaveValue('7d')
    expect(
      screen.getByText('以下设置已被服务器配置固定，你对它们的修改已丢弃：auth.session_ttl'),
    ).toBeInTheDocument()
    expect(screen.getByText('1 项未保存的更改')).toBeInTheDocument()
    expect(screen.getByLabelText('auth.registration')).toHaveValue('open')
  })
})

describe('settings page history and guards', () => {
  it('opens the history of a key, newest first, with resets marked', async () => {
    const history = {
      items: [
        {
          version: 2,
          old_value: '60d',
          new_value: '30d',
          reset: true,
          updated_by: 7,
          updated_by_username: 'alice',
          updated_at: 1_700_000_100_000,
          reason: null,
        },
        {
          version: 1,
          old_value: null,
          new_value: '60d',
          reset: false,
          updated_by: 7,
          updated_by_username: 'alice',
          updated_at: 1_700_000_000_000,
          reason: 'longer sessions',
        },
      ],
      total: 2,
      page: 1,
      page_size: 20,
    }
    const { user, fetchSpy } = renderApp('/settings', {
      api: {
        [`GET ${SETTINGS_PATH}`]: { body: sampleSettings() },
        [`GET ${SETTINGS_PATH}/${TTL}/history`]: { body: history },
      },
    })
    await screen.findByText(TTL)

    const row = screen.getByLabelText(TTL).closest('li') as HTMLElement
    await user.click(within(row).getByRole('button', { name: '历史' }))

    const drawer = await screen.findByRole('dialog', { name: '修改历史' })
    const items = await within(drawer).findAllByRole('listitem')
    expect(items).toHaveLength(2)
    const [newest, oldest] = items
    if (newest === undefined || oldest === undefined) throw new Error('two entries expected')
    expect(within(newest).getByText(/60d → 30d/)).toBeInTheDocument()
    expect(within(newest).getByText('已恢复默认')).toBeInTheDocument()
    expect(within(oldest).getByText(/（未存储） → 60d/)).toBeInTheDocument()
    expect(within(oldest).getByText('longer sessions')).toBeInTheDocument()
    expect(fetchSpy).toHaveBeenCalledWith(
      expect.stringContaining(`/settings/${TTL}/history`),
      expect.anything(),
    )
  })

  it('asks before leaving the page with unsaved changes', async () => {
    const { user, router } = await openPage(sampleSettings())

    await user.selectOptions(screen.getByLabelText('auth.registration'), 'open')
    void router.navigate({ to: '/groups' })

    const dialog = await screen.findByRole('alertdialog', { name: '不保存就离开？' })
    await user.click(within(dialog).getByRole('button', { name: '取消' }))
    expect(router.state.location.pathname).toBe('/settings')
    expect(screen.getByLabelText('auth.registration')).toHaveValue('open')

    void router.navigate({ to: '/groups' })
    const again = await screen.findByRole('alertdialog', { name: '不保存就离开？' })
    await user.click(within(again).getByRole('button', { name: '离开此页' }))
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
