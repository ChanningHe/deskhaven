import { act, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeDeps } from '../connect/fake-deps'
import { NO_STATS } from '../session/stats'
import { SAMPLE_VERSION, type VideoSample } from '../video/sample-export'
import { RemoteDepsContext } from './deps-context'
import { openMenu } from './menus/testing'
import { RemotePage } from './remote-page'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
})

async function activePage() {
  const f = fakeDeps()
  const view = renderWithProviders(
    <RemoteDepsContext.Provider value={f.deps}>
      <RemotePage deviceId="123456789" />
    </RemoteDepsContext.Provider>,
  )
  await waitFor(() => {
    expect(f.sessions).toHaveLength(1)
  })
  const [session] = f.sessions
  if (!session) throw new Error('no session started')
  act(() => {
    session.activate()
  })
  return { ...view, session, f }
}

async function showStats(user: Awaited<ReturnType<typeof activePage>>['user']) {
  await openMenu(user, '显示')
  await user.click(screen.getByRole('menuitemcheckbox', { name: '统计信息' }))
}

describe('stats overlay', () => {
  it('stays on after a reconnect', async () => {
    const { user, session, f } = await activePage()
    await showStats(user)
    act(() => {
      session.end({ by: 'link', reason: 'relay connection closed (code 1006)' })
    })
    expect(screen.queryByLabelText('连接统计')).toBeNull()
    await waitFor(
      () => {
        expect(f.sessions).toHaveLength(2)
      },
      { timeout: 3000 },
    )
    act(() => {
      f.sessions[1]?.activate()
    })
    expect(screen.getByLabelText('连接统计')).toBeInTheDocument()
  })

  it('is off until the display menu turns it on, and off again', async () => {
    const { user } = await activePage()
    const item = () => screen.getByRole('menuitemcheckbox', { name: '统计信息' })
    await openMenu(user, '显示')
    expect(item()).toHaveAttribute('aria-checked', 'false')
    expect(screen.queryByLabelText('连接统计')).toBeNull()

    await user.click(item())
    expect(screen.getByLabelText('连接统计')).toBeInTheDocument()
    await openMenu(user, '显示')
    expect(item()).toHaveAttribute('aria-checked', 'true')

    await user.click(item())
    expect(screen.queryByLabelText('连接统计')).toBeNull()
  })

  it("shows the session's figures and the controller's transport, raw values readable", async () => {
    const { user, session } = await activePage()
    await showStats(user)
    act(() => {
      session.stats.set({
        ...NO_STATS,
        fps: 29.5,
        kbps: 1840,
        delayMs: 83,
        keyRequests: 2,
        width: 1280,
        height: 800,
        codec: 'vp09.00.10.08',
      })
    })
    const overlay = screen.getByLabelText('连接统计')
    const stat = (name: string) => overlay.querySelector(`[data-stat="${name}"]`)
    expect(stat('fps')).toHaveAttribute('data-value', '29.5')
    expect(stat('kbps')).toHaveTextContent('1840 kbps')
    expect(stat('delayMs')).toHaveTextContent('83 ms')
    expect(stat('decodeMs')).toHaveAttribute('data-value', '')
    expect(stat('keyRequests')).toHaveAttribute('data-value', '2')
    expect(stat('size')).toHaveTextContent('1280×800')
    expect(stat('codec')).toHaveTextContent('vp09.00.10.08')
    expect(within(overlay).getByText('中继')).toBe(stat('transport'))
    expect(stat('quality')).toHaveAttribute('data-value', '')
  })

  it.each([
    ['balanced', 'balanced', '均衡'],
    [{ custom: 18 }, '18', '自定义 18'],
  ] as const)('shows the quality the session asks for: %o', async (quality, value, text) => {
    const { user, session } = await activePage()
    await showStats(user)
    act(() => {
      session.stats.set({ ...NO_STATS, quality })
    })
    const stat = screen.getByLabelText('连接统计').querySelector('[data-stat="quality"]')
    expect(stat).toHaveAttribute('data-value', value)
    expect(stat).toHaveTextContent(text)
  })

  it('shows the fps asked of the peer and the decode estimate after fps', async () => {
    const { user, session } = await activePage()
    await showStats(user)
    act(() => {
      session.stats.set({ ...NO_STATS, fps: 29.5, sentFps: 30, decodeFps: null })
    })
    const overlay = screen.getByLabelText('连接统计')
    const stats = [...overlay.querySelectorAll('dd')].map((dd) => dd.dataset.stat)
    expect(stats.slice(stats.indexOf('fps'), stats.indexOf('fps') + 3)).toEqual([
      'fps',
      'sentFps',
      'decodeFps',
    ])
    const stat = (name: string) => overlay.querySelector(`[data-stat="${name}"]`)
    expect(stat('sentFps')).toHaveAttribute('data-value', '30')
    expect(stat('sentFps')).toHaveTextContent('30')
    expect(stat('decodeFps')).toHaveAttribute('data-value', '')
    expect(stat('decodeFps')).toHaveTextContent('–')
    expect(within(overlay).getByText('请求帧率')).toBeInTheDocument()
    expect(within(overlay).getByText('解码估算')).toBeInTheDocument()

    act(() => {
      session.stats.set({ ...NO_STATS, sentFps: null, decodeFps: 48 })
    })
    expect(stat('sentFps')).toHaveAttribute('data-value', '')
    expect(stat('sentFps')).toHaveTextContent('–')
    expect(stat('decodeFps')).toHaveAttribute('data-value', '48')
    expect(stat('decodeFps')).toHaveTextContent('48')
  })
})

const SAMPLE: VideoSample = {
  version: SAMPLE_VERSION,
  clientVersion: '1.5.0',
  codec: 'vp9s',
  complete: true,
  peer: { platform: 'Linux', version: '1.5.0' },
  display: { width: 1280, height: 800 },
  frames: [{ key: true, pts: 16, data: 'AAAAAQ==' }],
}

describe('video sample export', () => {
  it('records on click, shows how far it got, and cancels on a second click', async () => {
    const { user, session } = await activePage()
    await showStats(user)
    const overlay = screen.getByLabelText('连接统计')
    await user.click(within(overlay).getByRole('button', { name: '导出视频样本' }))
    expect(session.sampleRequests).toBe(1)
    expect(within(overlay).getByRole('button', { name: /等待关键帧/ })).toBeInTheDocument()

    act(() => {
      session.sampling.set({ kind: 'recording', frames: 12 })
    })
    const recording = within(overlay).getByRole('button', { name: /12\/30/ })
    await user.click(recording)
    expect(session.sampling.get()).toEqual({ kind: 'idle' })
    expect(within(overlay).getByRole('button', { name: '导出视频样本' })).toBeInTheDocument()
  })

  it('saves the recorded sample as a JSON file named after the codec', async () => {
    const urls: Blob[] = []
    const revoke = vi.fn()
    Object.assign(URL, {
      createObjectURL: (blob: Blob) => {
        urls.push(blob)
        return 'blob:sample'
      },
      revokeObjectURL: revoke,
    })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      const { user, session } = await activePage()
      await showStats(user)
      await user.click(screen.getByRole('button', { name: '导出视频样本' }))
      act(() => {
        session.finishSample(SAMPLE)
      })
      await waitFor(() => {
        expect(click).toHaveBeenCalledTimes(1)
      })
      const anchor = click.mock.contexts[0] as HTMLAnchorElement
      expect(anchor.download).toMatch(/^video-sample-vp9s-\d{8}-\d{6}\.json$/)
      expect(anchor.href).toBe('blob:sample')
      const [blob] = urls
      if (!blob) throw new Error('no blob')
      expect(JSON.parse(await blob.text())).toEqual(SAMPLE)
      expect(revoke).not.toHaveBeenCalled()
    } finally {
      click.mockRestore()
    }
  })

  it('saves nothing when cancelled', async () => {
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    try {
      const { user, session } = await activePage()
      await showStats(user)
      await user.click(screen.getByRole('button', { name: '导出视频样本' }))
      act(() => {
        session.finishSample(null)
      })
      await settleUi()
      expect(click).not.toHaveBeenCalled()
    } finally {
      click.mockRestore()
    }
  })
})

async function settleUi() {
  await act(async () => {
    await Promise.resolve()
  })
}
