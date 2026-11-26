import { act, fireEvent, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { OPENED, type FakeSession } from '../connect/fake-session'
import { SETTINGS_KEY } from '../settings/settings'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

const DEVICE = '123456789'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

const LEAVE = <a href="/">返回控制台</a>

function page(f: ReturnType<typeof fakeDeps>) {
  return (
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId={DEVICE} leave={LEAVE} />
    </RemoteDepsContext.Provider>
  )
}

/** After userEvent.setup, which installs a clipboard of its own. */
function stubClipboard(clipboard: Pick<Clipboard, 'readText' | 'writeText'>) {
  Object.defineProperty(window.navigator, 'clipboard', { value: clipboard, configurable: true })
}

async function open(f = fakeDeps()) {
  const view = renderWithProviders(page(f))
  await waitFor(() => {
    expect(f.sessions).toHaveLength(1)
  })
  const [session] = f.sessions
  if (!session) throw new Error('no session started')
  return { ...view, f, session }
}

/** Focuses the screen's keyboard input, as a click on the screen does. */
function focusScreen(): HTMLTextAreaElement {
  const keys = screen.getByRole<HTMLTextAreaElement>('textbox', { name: /远程屏幕/ })
  act(() => {
    keys.focus()
  })
  return keys
}

/** A cancelable key event on `target` at `time` (KeyboardEvent.timeStamp). */
function key(target: Element, type: 'keydown' | 'keyup', init: KeyboardEventInit, time = 0) {
  const event = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init })
  Object.defineProperty(event, 'timeStamp', { value: time })
  act(() => {
    target.dispatchEvent(event)
  })
  return event
}

function set(session: FakeSession, state: Parameters<FakeSession['state']['set']>[0]) {
  act(() => {
    session.state.set(state)
  })
}

describe('remote page: logging in', () => {
  it('asks for the device password and sends it', async () => {
    const { user, session } = await open()

    set(session, { kind: 'need_password' })
    await user.type(screen.getByLabelText('密码'), 'hunter2')
    await user.click(screen.getByRole('button', { name: '连接' }))
    expect(session.logins).toEqual(['hunter2'])
    expect(screen.queryByLabelText('验证码')).toBeNull()

    set(session, { kind: 'wrong_password' })
    expect(screen.getByRole('alert')).toBeInTheDocument()
  })

  it('asks for the two-factor code only when the device wants one', async () => {
    const { user, session } = await open()
    set(session, { kind: 'need_2fa', wrongCode: false })
    expect(screen.queryByLabelText('密码')).toBeNull()
    await user.type(screen.getByLabelText('验证码'), '123456')
    await user.click(screen.getByRole('button', { name: '验证' }))
    expect(session.codes).toEqual(['123456'])

    set(session, { kind: 'need_2fa', wrongCode: true })
    expect(screen.getByRole('alert')).toBeInTheDocument()
  })

  it('asks which Windows session to control, the current one picked first', async () => {
    const { user, session } = await open()
    set(session, {
      kind: 'choose_session',
      sessions: [
        { sid: 1, name: 'Console' },
        { sid: 2, name: 'RDP: pve' },
      ],
      current: 2,
    })
    const select = screen.getByLabelText('会话')
    expect(select).toHaveValue('2')

    await user.selectOptions(select, '1')
    await user.click(screen.getByRole('button', { name: '继续' }))
    expect(session.chosenSessions).toEqual([1])
  })

  it('waits for the person at the device, and gives up on cancel', async () => {
    const { user, session, f } = await open()
    set(session, { kind: 'awaiting_approval' })
    await user.click(screen.getByRole('button', { name: '取消' }))
    expect(session.closed).toBe(1)
    expect(f.closeSession).toHaveBeenCalledWith(OPENED.session_id)
  })
})

describe('remote page: failures', () => {
  it('names the relay the device uses, and tries again on request', async () => {
    const error = new ApiError(409, {
      code: 'foreign_relay',
      details: { relay_server: 'relay.other.example' },
    })
    const f = fakeDeps(() => Promise.reject(error))
    const { user } = renderWithProviders(page(f))
    expect(await screen.findByRole('alert')).toHaveTextContent('relay.other.example')
    await user.click(screen.getByRole('button', { name: '重试' }))
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it('reconnects with a new server session after the session ended', async () => {
    const { user, session, f } = await open()
    act(() => {
      session.end({ by: 'link', reason: 'timeout' })
    })
    expect(f.closeSession).toHaveBeenCalledTimes(1)
    await user.click(screen.getByRole('button', { name: '重新连接' }))
    await waitFor(() => {
      expect(f.sessions).toHaveLength(2)
    })
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })
})

describe('remote page: leaving', () => {
  it('hangs up and closes the server session when the page unmounts', async () => {
    const { unmount, session, f } = await open()
    unmount()
    expect(session.closed).toBe(1)
    expect(f.closeSession).toHaveBeenCalledWith(OPENED.session_id)
  })

  it('hangs up and closes the server session when the tab goes away', async () => {
    const { session, f } = await open()
    fireEvent(window, new Event('pagehide'))
    expect(session.closed).toBe(1)
    expect(f.closeSession).toHaveBeenCalledTimes(1)
  })
})

describe('remote page: active session', () => {
  it('greys out the controls the device turned off', async () => {
    const { user, session } = await open()
    act(() => {
      session.activate()
      session.permissions.set({ keyboard: false, clipboard: false, restart: true, audio: true })
    })
    await openMenu(user, '输入')
    expect(screen.getByRole('menuitem', { name: '发送按键' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
    for (const mode of screen.getAllByRole('menuitemradio')) {
      expect(mode).toHaveAttribute('aria-disabled', 'true')
    }
    await user.keyboard('{Escape}')
    await openMenu(user, '剪贴板')
    expect(screen.getByRole('menuitem', { name: '发送剪贴板' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
    expect(screen.getByRole('menuitem', { name: '远端剪贴板' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
  })

  it('takes keys while the screen has focus and lets them all go on blur', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const keys = focusScreen()
    const down = new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', cancelable: true })
    keys.dispatchEvent(down)
    fireEvent.keyDown(keys, { key: 'F12', code: 'F12' })
    fireEvent.contextMenu(screen.getByTestId('remote-screen'))
    act(() => {
      keys.blur()
    })
    expect(down.defaultPrevented).toBe(true)
    expect(session.inputs).toMatchObject([
      { kind: 'key', down: true, key: 'a', code: 'KeyA' },
      { kind: 'release' },
    ])
  })

  it('shows what the device copied, without writing the local clipboard when sync is off', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    const f = fakeDeps()
    f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: { clipboardSync: false } }))
    const { user, session } = await open(f)
    stubClipboard({ writeText, readText: vi.fn() })
    act(() => {
      session.activate()
    })
    act(() => {
      session.remoteClipboard.set('copied over there')
    })
    expect(writeText).not.toHaveBeenCalled()
    await openMenu(user, '剪贴板')
    await user.click(screen.getByRole('menuitem', { name: '远端剪贴板' }))
    expect(screen.getByRole('textbox', { name: '设备上复制的内容' })).toHaveValue(
      'copied over there',
    )
    await user.click(screen.getByRole('button', { name: '复制' }))
    expect(writeText).toHaveBeenCalledWith('copied over there')
  })

  it('sends the local clipboard, or offers a box to paste into when it may not read it', async () => {
    const readText = vi
      .fn<() => Promise<string>>()
      .mockResolvedValueOnce('from here')
      .mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
    const { user, session } = await open()
    stubClipboard({ readText, writeText: vi.fn() })
    act(() => {
      session.activate()
    })
    await openMenu(user, '剪贴板')
    await user.click(screen.getByRole('menuitem', { name: '发送剪贴板' }))
    expect(session.pasted).toEqual(['from here'])

    await openMenu(user, '剪贴板')
    await user.click(screen.getByRole('menuitem', { name: '发送剪贴板' }))
    await user.type(await screen.findByLabelText('要发送的文本'), 'typed')
    await user.click(screen.getByRole('button', { name: '发送' }))
    expect(session.pasted).toEqual(['from here', 'typed'])
  })
  it('sends what an IME commits as text, and no keys while it composes', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const keys = focusScreen()
    const start = key(keys, 'keydown', { key: 'Process', code: 'KeyN' })
    keys.dispatchEvent(new CompositionEvent('compositionstart', { data: '' }))
    const composing = key(keys, 'keydown', { key: 'i', code: 'KeyI', isComposing: true })
    key(keys, 'keyup', { key: 'i', code: 'KeyI', isComposing: true })
    keys.value = '你'
    keys.dispatchEvent(new CompositionEvent('compositionend', { data: '你' }))
    const a = key(keys, 'keydown', { key: 'a', code: 'KeyA' })
    expect(start.defaultPrevented).toBe(false)
    expect(composing.defaultPrevented).toBe(false)
    expect(a.defaultPrevented).toBe(true)
    expect(keys).toHaveValue('')
    expect(session.inputs).toEqual([
      { kind: 'text', text: '你' },
      expect.objectContaining({ kind: 'key', down: true, key: 'a', code: 'KeyA' }),
    ])
  })

  it('sends Esc on to the device once the limits popover has closed', async () => {
    const { user, session } = await open()
    act(() => {
      session.activate()
    })
    await user.click(screen.getByRole('button', { name: '使用限制' }))
    expect(await screen.findByRole('dialog', { name: '使用限制' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull()
    })
    const keys = focusScreen()
    key(keys, 'keydown', { key: 'Escape', code: 'Escape' }, 100)
    key(keys, 'keyup', { key: 'Escape', code: 'Escape' }, 150)
    expect(document.activeElement).toBe(keys)
    expect(session.inputs.map((i) => (i.kind === 'key' ? `${i.key} ${i.down}` : i.kind))).toEqual([
      'Escape true',
      'Escape false',
    ])
  })

  it('leaves the screen for the toolbar on Esc twice, letting go of held keys', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const keys = focusScreen()
    const escape = { key: 'Escape', code: 'Escape' }
    key(keys, 'keydown', { key: 'Tab', code: 'Tab' }, 100)
    key(keys, 'keydown', escape, 200)
    key(keys, 'keyup', escape, 260)
    expect(document.activeElement).toBe(keys)
    const second = key(keys, 'keydown', escape, 1000)
    expect(document.activeElement).toBe(keys)
    key(keys, 'keyup', escape, 1050)
    const third = key(keys, 'keydown', escape, 1300)

    expect(second.defaultPrevented).toBe(true)
    expect(third.defaultPrevented).toBe(true)
    expect(document.activeElement).toBe(screen.getByRole('button', { name: '显示' }))
    expect(session.inputs.map((i) => (i.kind === 'key' ? `${i.key} ${i.down}` : i.kind))).toEqual([
      'Tab true',
      'Escape true',
      'Escape false',
      'Escape true',
      'Escape false',
      'release',
    ])
  })
})
