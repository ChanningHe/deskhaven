import { act, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { fakeAudioDeps } from '../audio/testing'
import { fakeDeps } from '../connect/fake-deps'
import type { FakeSession } from '../connect/fake-session'
import { OptionMessage_BoolOption } from '../proto/gen/message_pb.js'
import { ALL_ALLOWED } from '../session/permissions'
import { SETTINGS_KEY } from '../settings/settings'
import { RemoteDepsContext } from './deps-context'
import { RemotePage } from './remote-page'

beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
  HTMLElement.prototype.setPointerCapture = vi.fn()
})

async function activePage({
  audio = fakeAudioDeps(),
  stored,
}: { audio?: ReturnType<typeof fakeAudioDeps>; stored?: boolean } = {}) {
  const f = fakeDeps()
  if (stored !== undefined) {
    f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: { audio: stored } }))
  }
  const deps = { ...f.deps, audio: audio.deps }
  const view = renderWithProviders(
    <RemoteDepsContext.Provider value={deps}>
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
  session.patches.length = 0
  return { ...view, f, session, contexts: audio.contexts }
}

const sound = (name: string) => screen.findByRole('button', { name })

const { Yes, No } = OptionMessage_BoolOption
const audioSent = (session: FakeSession) =>
  session.patches.flatMap((p) => (p.peer?.disableAudio ? [p.peer.disableAudio] : []))

describe('sound button', () => {
  it('starts off: one click opens and resumes the context and asks the peer for audio', async () => {
    const { user, session, contexts } = await activePage()
    await user.click(await sound('开启声音'))
    expect(contexts).toHaveLength(1)
    expect(contexts[0]?.resumed).toBe(1)
    expect(session.patches.map((p) => p.peer?.disableAudio)).toEqual([OptionMessage_BoolOption.No])
    expect(session.audio.output?.running).toBe(true)
    expect(await sound('关闭声音')).toBeInTheDocument()
  })

  it('turning it off tells the peer, closes the context and detaches the session', async () => {
    const { user, session, contexts } = await activePage()
    await user.click(await sound('开启声音'))
    session.patches.length = 0
    await user.click(await sound('关闭声音'))
    expect(session.patches.map((p) => p.peer?.disableAudio)).toEqual([OptionMessage_BoolOption.Yes])
    expect(contexts[0]?.state).toBe('closed')
    expect(session.audio.output).toBeNull()
    expect(await sound('开启声音')).toBeInTheDocument()
  })

  it('stored on, no gesture yet: asks for a click, which resumes the same context', async () => {
    const { user, session, contexts } = await activePage({ stored: true })
    const button = await sound('点击开启声音')
    expect(contexts).toHaveLength(1)
    expect(session.audio.output?.running).toBe(false)
    expect(audioSent(session)).toEqual([])
    await user.click(button)
    expect(contexts).toHaveLength(1)
    expect(contexts[0]?.state).toBe('running')
    expect(await sound('关闭声音')).toBeInTheDocument()
    expect(audioSent(session)).toEqual([No])
  })

  it('tells the peer to stop when the browser suspends the context', async () => {
    const { user, session, contexts } = await activePage()
    await user.click(await sound('开启声音'))
    act(() => {
      contexts[0]?.browserSets('suspended')
    })
    expect(await sound('点击开启声音')).toBeInTheDocument()
    expect(audioSent(session)).toEqual([No, Yes])
  })

  it.each([
    ['the browser cannot decode Opus', fakeAudioDeps({ opus: false })],
    ['the probe fails', fakeAudioDeps({ opus: 'throws' })],
  ])('is disabled when %s, and stays so on a click', async (_, audio) => {
    const { user, session, contexts } = await activePage({ audio })
    const button = await sound('此浏览器无法播放设备声音')
    await waitFor(() => {
      expect(button).toHaveAttribute('aria-disabled', 'true')
    })
    await user.click(button)
    expect(contexts).toHaveLength(0)
    expect(session.patches).toEqual([])
  })

  it('is disabled while the device does not allow audio', async () => {
    const { user, session, contexts } = await activePage()
    await sound('开启声音')
    act(() => {
      session.permissions.set({ ...ALL_ALLOWED, audio: false })
    })
    const button = await sound('设备不允许传输声音')
    expect(button).toHaveAttribute('aria-disabled', 'true')
    await user.click(button)
    expect(contexts).toHaveLength(0)
    act(() => {
      session.permissions.set(ALL_ALLOWED)
    })
    expect(await sound('开启声音')).not.toHaveAttribute('aria-disabled', 'true')
  })

  it('is disabled when the browser cannot decode the format the device told', async () => {
    const { session } = await activePage()
    await sound('开启声音')
    act(() => {
      session.audio.state.set('unsupported')
    })
    expect(await sound('此浏览器无法播放设备声音')).toHaveAttribute('aria-disabled', 'true')
  })

  it('stops playing while the device takes the audio permission away', async () => {
    const { user, session } = await activePage()
    await user.click(await sound('开启声音'))
    const output = session.audio.output
    act(() => {
      session.permissions.set({ ...ALL_ALLOWED, audio: false })
    })
    expect(session.audio.output).toBeNull()
    act(() => {
      session.permissions.set(ALL_ALLOWED)
    })
    expect(session.audio.output).toBe(output)
  })

  it('says so when decoding gave up, and a click turns the sound off', async () => {
    const { user, session } = await activePage()
    await user.click(await sound('开启声音'))
    act(() => {
      session.audio.state.set('failed')
    })
    await user.click(await sound('声音解码失败，点击关闭'))
    expect(session.patches.at(-1)?.peer?.disableAudio).toBe(OptionMessage_BoolOption.Yes)
  })

  it('plays the next session of the page into the same context', async () => {
    const { user, f, session } = await activePage()
    await user.click(await sound('开启声音'))
    const output = session.audio.output
    act(() => {
      session.end({ by: 'peer', reason: 'gone' })
    })
    expect(session.audio.output).toBeNull()
    await user.click(await screen.findByRole('button', { name: '重新连接' }))
    await waitFor(() => {
      expect(f.sessions).toHaveLength(2)
    })
    act(() => {
      f.sessions[1]?.activate()
    })
    await sound('关闭声音')
    expect(f.sessions[1]?.audio.output).toBe(output)
    const next = f.sessions[1]
    if (!next) throw new Error('no second session')
    expect(audioSent(next)).toEqual([No])
  })
})
