import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from '@/api/api-error'
import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { OPENED, type FakeSession } from '../connect/fake-session'
import { KeyboardMode } from '../proto/gen/message_pb.js'
import { RemoteDepsContext } from './deps-context'
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
    ['too_many_sessions', 429, '远程会话过多'],
  ] as const)('explains %s', async (code, status, text) => {
    const f = fakeDeps(() => Promise.reject(new ApiError(status, { code })))
    renderWithProviders(page(f))
    expect(await screen.findByRole('alert')).toHaveTextContent(text)
  })

  it('says why the session ended and reconnects with a new server session', async () => {
    const { user, session, f } = await open()
    act(() => {
      session.end('timeout')
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
    const { session } = await open()
    act(() => {
      session.activate()
      session.permissions.set({ keyboard: false, clipboard: false })
    })
    const bar = screen.getByRole('toolbar', { name: '会话控制' })
    expect(within(bar).getByRole('button', { name: 'Ctrl+Alt+Del' })).toBeDisabled()
    expect(within(bar).getByLabelText('键盘模式')).toBeDisabled()
    expect(within(bar).getByRole('button', { name: '发送剪贴板' })).toBeDisabled()
    expect(within(bar).getByRole('button', { name: '远端剪贴板' })).toBeDisabled()
    expect(screen.getByText('设备已关闭键盘和鼠标控制。')).toBeInTheDocument()
    expect(screen.getByText('设备已关闭剪贴板。')).toBeInTheDocument()
  })

  it('sends Ctrl+Alt+Del, keyboard mode and display changes', async () => {
    const { user, session } = await open()
    act(() => {
      session.activate(2)
    })
    await user.click(screen.getByRole('button', { name: 'Ctrl+Alt+Del' }))
    await user.selectOptions(screen.getByLabelText('键盘模式'), '翻译')
    await user.selectOptions(screen.getByLabelText('显示器'), '2')
    expect(session.ctrlAltDels).toBe(1)
    expect(session.modes).toEqual([KeyboardMode.Translate])
    expect(session.displays).toEqual([1])
  })

  it('takes keys while the screen has focus and lets them all go on blur', async () => {
    const { session } = await open()
    act(() => {
      session.activate()
    })
    const screenCanvas = screen.getByRole('application', { name: /远程屏幕/ })
    act(() => {
      screenCanvas.focus()
    })
    const down = new KeyboardEvent('keydown', { key: 'a', code: 'KeyA', cancelable: true })
    screenCanvas.dispatchEvent(down)
    fireEvent.keyDown(screenCanvas, { key: 'F12', code: 'F12' })
    fireEvent.contextMenu(screenCanvas)
    act(() => {
      screenCanvas.blur()
    })
    expect(down.defaultPrevented).toBe(true)
    expect(session.inputs).toMatchObject([
      { kind: 'key', down: true, key: 'a', code: 'KeyA' },
      { kind: 'release' },
    ])
  })

  it('shows what the device copied without writing the local clipboard', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    const { user, session } = await open()
    stubClipboard({ writeText, readText: vi.fn() })
    act(() => {
      session.activate()
      session.remoteClipboard.set('copied over there')
    })
    expect(writeText).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: '远端剪贴板' }))
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
    await user.click(screen.getByRole('button', { name: '发送剪贴板' }))
    expect(session.pasted).toEqual(['from here'])
    expect(await screen.findByText('已发送到设备剪贴板。')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '发送剪贴板' }))
    await user.type(await screen.findByLabelText('要发送的文本'), 'typed')
    await user.click(screen.getByRole('button', { name: '发送' }))
    expect(session.pasted).toEqual(['from here', 'typed'])
  })
})
