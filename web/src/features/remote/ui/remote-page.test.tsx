import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { OPENED, type FakeSession } from '../connect/fake-session'
import { KeyboardMode } from '../proto/gen/message_pb.js'
import { SETTINGS_KEY } from '../settings/settings'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

const DEVICE = '123456789'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

function page(f: ReturnType<typeof fakeDeps>) {
  return (
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId={DEVICE} />
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
    expect(screen.getByRole('status')).toHaveTextContent('正在连接设备…')

    set(session, { kind: 'need_password' })
    await user.type(screen.getByLabelText('密码'), 'hunter2')
    await user.click(screen.getByRole('button', { name: '连接' }))
    expect(session.logins).toEqual(['hunter2'])
    expect(screen.queryByLabelText('验证码')).toBeNull()

    set(session, { kind: 'wrong_password' })
    expect(screen.getByRole('alert')).toHaveTextContent('密码错误，请重试。')
  })

  it('asks for the two-factor code only when the device wants one', async () => {
    const { user, session } = await open()
    set(session, { kind: 'need_2fa', wrongCode: false })
    expect(screen.queryByLabelText('密码')).toBeNull()
    await user.type(screen.getByLabelText('验证码'), '123456')
    await user.click(screen.getByRole('button', { name: '验证' }))
    expect(session.codes).toEqual(['123456'])

    set(session, { kind: 'need_2fa', wrongCode: true })
    expect(screen.getByRole('alert')).toHaveTextContent('验证码错误，请重试。')
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
    expect(within(select).getByRole('option', { name: 'RDP: pve（当前）' })).toBeInTheDocument()
    expect(screen.queryByText(/断开本次连接/)).toBeNull()

    await user.selectOptions(select, '1')
    expect(screen.getByText(/断开本次连接/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '继续' }))
    expect(session.chosenSessions).toEqual([1])

    act(() => {
      session.end({ by: 'session_switch', reason: 'relay connection closed (code 1006)' })
    })
    expect(screen.getByRole('status')).toHaveTextContent('连接已中断，正在重连（第 1 次）…')
  })

  it('waits for the person at the device, and gives up on cancel', async () => {
    const { user, session, f } = await open()
    set(session, { kind: 'awaiting_approval' })
    expect(screen.getByText('需要设备前的人接受此连接。')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '取消' }))
    expect(session.closed).toBe(1)
    expect(f.closeSession).toHaveBeenCalledWith(OPENED.session_id)
    expect(screen.getByRole('alert')).toHaveTextContent('你已结束会话。')
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
    expect(screen.getByRole('heading', { name: '无法连接' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '重试' }))
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })

  it.each([
    ['web_client_disabled', 403, '此服务器未开启浏览器远程'],
    ['device_offline', 409, '设备离线'],
    ['too_many_sessions', 429, '远程连接过多'],
  ] as const)('explains %s', async (code, status, text) => {
    const f = fakeDeps(() => Promise.reject(new ApiError(status, { code })))
    renderWithProviders(page(f))
    expect(await screen.findByRole('alert')).toHaveTextContent(text)
  })

  it('says why the session ended and reconnects with a new server session', async () => {
    const { user, session, f } = await open()
    act(() => {
      session.end({ by: 'link', reason: 'timeout' })
    })
    expect(screen.getByRole('heading', { name: '已断开' })).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('设备未在规定时间内完成连接。')
    expect(f.closeSession).toHaveBeenCalledTimes(1)
    await user.click(screen.getByRole('button', { name: '重新连接' }))
    await waitFor(() => {
      expect(f.sessions).toHaveLength(2)
    })
    expect(f.openSession).toHaveBeenCalledTimes(2)
  })
})

describe('remote page: reconnecting', () => {
  it('reconnects by itself after the link drops, and goes manual on cancel', async () => {
    const { user, session, f } = await open()
    act(() => {
      session.activate()
    })
    act(() => {
      session.end({ by: 'link', reason: 'relay connection closed (code 1006)' })
    })
    expect(screen.getByRole('status')).toHaveTextContent('连接已中断，正在重连（第 1 次）…')
    expect(screen.queryByLabelText('密码')).toBeNull()

    await user.click(screen.getByRole('button', { name: '取消' }))
    expect(screen.getByRole('alert')).toHaveTextContent('你已结束会话。')
    await new Promise((resolve) => setTimeout(resolve, 1100))
    expect(f.openSession).toHaveBeenCalledTimes(1)
  })

  it("shows the peer closing the session as the peer's, and does not reconnect", async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    act(() => {
      session.end({ by: 'peer', reason: 'closed' })
    })
    expect(screen.getByRole('alert')).toHaveTextContent('会话已结束：closed')
    expect(screen.getByRole('button', { name: '重新连接' })).toBeInTheDocument()
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
  it('names the link the session won the race on', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const status = screen.getByText(/已连接/)
    expect(status).toHaveTextContent('中继')
    expect(status).not.toHaveTextContent('WebRTC')
  })

  it('greys out the controls the device turned off', async () => {
    const { user, session } = await open()
    act(() => {
      session.activate()
      session.permissions.set({ keyboard: false, clipboard: false, restart: true })
    })
    expect(screen.getByRole('toolbar', { name: '会话控制' })).toBeInTheDocument()
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
    expect(screen.getByText('设备已关闭键盘和鼠标控制。')).toBeInTheDocument()
    expect(screen.getByText('设备已关闭剪贴板。')).toBeInTheDocument()
  })

  it('sends keyboard mode changes from the input menu', async () => {
    const { user, session } = await open()
    act(() => {
      session.activate(2)
    })
    await openMenu(user, '输入')
    expect(screen.getByRole('menuitemradio', { name: '映射' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    await user.click(screen.getByRole('menuitemradio', { name: '翻译' }))
    expect(session.modes).toEqual([KeyboardMode.Translate])
  })

  it('goes full screen and back', async () => {
    const requestFullscreen = vi.fn(() => Promise.resolve())
    const exitFullscreen = vi.fn(() => Promise.resolve())
    document.documentElement.requestFullscreen = requestFullscreen
    document.exitFullscreen = exitFullscreen
    const { user, session } = await open()
    act(() => {
      session.activate()
    })
    await user.click(screen.getByRole('button', { name: '全屏' }))
    expect(requestFullscreen).toHaveBeenCalledTimes(1)
    Object.defineProperty(document, 'fullscreenElement', {
      value: document.documentElement,
      configurable: true,
    })
    fireEvent(document, new Event('fullscreenchange'))
    await user.click(screen.getByRole('button', { name: '退出全屏' }))
    expect(exitFullscreen).toHaveBeenCalledTimes(1)
    Object.defineProperty(document, 'fullscreenElement', { value: null, configurable: true })
    fireEvent(document, new Event('fullscreenchange'))
    expect(screen.getByRole('button', { name: '全屏' })).toBeInTheDocument()
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
    expect(await screen.findByText('已发送到设备剪贴板。')).toBeInTheDocument()

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

  it('leaves the screen for the toolbar on Esc twice, letting go of held keys', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const keys = focusScreen()
    expect(keys).toHaveAccessibleDescription(/连按两次 Esc/)
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
