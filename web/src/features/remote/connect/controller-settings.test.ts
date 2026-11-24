import { describe, expect, it, vi } from 'vitest'

import { msg } from '../__fixtures__/harness'
import { ImageQuality, OptionMessage_BoolOption } from '../proto/gen/message_pb.js'
import { startSession, type Session } from '../session/session'
import { FakeClock, FakeHandshake, PEER_INFO } from '../session/testing'
import { FakeDecoders, vp9Frame } from '../video/fake-decoder'
import { BACKLOG } from '../video/fps-governor'
import { DEFAULT_SETTINGS, SETTINGS_KEY, type RemoteSettings } from '../settings/settings'
import { START_LEVEL, START_QUALITY } from '../video/quality-governor'
import { RemoteController } from './controller'
import { fakeDeps } from './fake-deps'

const DEVICE = '123456789'

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

async function connected(initial?: Partial<RemoteSettings>) {
  const f = fakeDeps()
  if (initial) {
    f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: initial }))
  }
  const controller = new RemoteController(DEVICE, f.deps)
  controller.connect()
  await settle()
  const session = f.sessions[0]
  if (!session) throw new Error('no session started')
  return { ...f, controller, session }
}

function loginOptionOf(f: Awaited<ReturnType<typeof connected>>, call = 0) {
  return f.startSession.mock.calls[call]?.[1].loginOption
}

describe('RemoteController settings', () => {
  it('starts from what the storage holds, field by field', async () => {
    const f = await connected({ fps: 45, scale: 'original', audio: 'loud' as unknown as boolean })
    expect(f.controller.settings.get()).toEqual({
      ...DEFAULT_SETTINGS,
      fps: 45,
      scale: 'original',
    })
  })

  it('saves a change and tells the subscribers', async () => {
    const f = await connected()
    let notified = 0
    f.controller.settings.subscribe(() => notified++)
    f.controller.updateSettings({ scale: 'original', clipboardSync: false })
    expect(f.controller.settings.get()).toMatchObject({ scale: 'original', clipboardSync: false })
    expect(notified).toBe(1)
    expect(new RemoteController(DEVICE, f.deps).settings.get()).toEqual(f.controller.settings.get())
  })

  it('works without a usable storage', () => {
    const f = fakeDeps()
    const controller = new RemoteController(DEVICE, {
      ...f.deps,
      storage: {
        getItem: () => {
          throw new Error('denied')
        },
        setItem: () => {
          throw new Error('quota')
        },
      } as unknown as Storage,
    })
    expect(controller.settings.get()).toEqual(DEFAULT_SETTINGS)
    expect(() => {
      controller.updateSettings({ fps: 20 })
    }).not.toThrow()
    expect(controller.settings.get().fps).toBe(20)
  })

  it("logs in at the governor's starting quality and the fps of the settings", async () => {
    const f = await connected({ quality: 'best', fps: 60 })
    expect(loginOptionOf(f)).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: START_LEVEL << 8,
      customFps: 60,
    })
    expect(f.startSession).toHaveBeenCalledWith(
      expect.anything(),
      expect.anything(),
      expect.anything(),
      60,
    )
  })

  it('tells an activated session no fps: it logged in with the one of the settings', async () => {
    const f = await connected({ fps: 60 })
    f.session.activate()
    expect(f.session.patches).toEqual([{ quality: DEFAULT_SETTINGS.quality }])
  })

  it('logs in again with the settings as they are by then', async () => {
    const f = await connected()
    f.controller.updateSettings({ quality: { custom: 70 }, fps: 20 })
    f.controller.connect()
    await settle()
    expect(loginOptionOf(f, 1)).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: START_LEVEL << 8,
      customFps: 20,
    })
  })
})

describe('RemoteController settings in a session', () => {
  it("tells an activated session the user's quality as the governor's cap", async () => {
    const f = await connected({ quality: 'best' })
    f.session.activate()
    expect(f.session.patches).toEqual([{ quality: 'best' }])
  })

  it('hands a quality change over as a cap, never as a peer option', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings({ quality: 'best' })
    expect(f.session.patches).toEqual([{ quality: 'best' }])
  })

  it('hands an fps change over as a ceiling, which is all the session may be told', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings({ fps: 45 })
    expect(f.session.patches).toEqual([{ maxFps: 45 }])
  })

  it('keeps the fps with a preset quality', async () => {
    const f = await connected({ quality: { custom: 60 }, fps: 50 })
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings({ quality: 'low' })
    expect(f.controller.settings.get()).toMatchObject({ quality: 'low', fps: 50 })
    expect(f.session.patches).toEqual([{ quality: 'low' }])
  })

  it.each([
    { scale: 'original' },
    { clipboardSync: false },
    { showRemoteCursor: false },
  ] satisfies Partial<RemoteSettings>[])('tells the session nothing about %o', async (patch) => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings(patch)
    expect(f.session.patches).toEqual([])
  })

  it('leaves a session that is not active alone, then catches it up when it is', async () => {
    const f = await connected({ fps: 60 })
    f.controller.updateSettings({ quality: { custom: 70 } })
    f.controller.updateSettings({ scale: 'original' })
    expect(f.session.patches).toEqual([])

    f.session.activate()
    expect(f.session.patches).toEqual([{ quality: { custom: 70 } }])
    f.session.state.set({ kind: 'active' })
    expect(f.session.patches).toHaveLength(1)
  })

  it('sends nothing once the login already carried the setting', async () => {
    const f = await connected({ quality: START_QUALITY })
    f.controller.updateSettings({ fps: 45 })
    f.controller.updateSettings({ fps: 30 })
    f.session.activate()
    expect(f.session.patches).toEqual([])
  })

  it('does not touch a closed session, or none at all', async () => {
    const idle = new RemoteController(DEVICE, fakeDeps().deps)
    expect(() => {
      idle.updateSettings({ quality: 'low' })
    }).not.toThrow()

    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.session.end({ by: 'peer', reason: 'gone' })
    f.controller.updateSettings({ quality: 'low' })
    expect(f.session.patches).toEqual([])
  })

  it('only reaches the session of the current connect', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.connect()
    await settle()
    f.controller.updateSettings({ quality: 'low' })
    expect(f.session.patches).toEqual([])
    expect(f.sessions[1]?.patches).toEqual([])
    f.sessions[1]?.activate()
    expect(f.sessions[1]?.patches).toEqual([{ quality: 'low' }])
    expect(f.session.patches).toEqual([])
  })
})

describe('RemoteController sound', () => {
  const { Yes, No } = OptionMessage_BoolOption
  const sent = (f: Awaited<ReturnType<typeof connected>>, session = f.session) =>
    session.patches.flatMap((p) => (p.peer?.disableAudio ? [p.peer.disableAudio] : []))

  it('logs in without audio even when the setting is on', async () => {
    const f = await connected({ audio: true })
    f.controller.setAudioRunning(true)
    expect(loginOptionOf(f)?.disableAudio).not.toBe(No)
  })

  it('asks for audio only once the setting is on and the AudioContext runs', async () => {
    const f = await connected({ audio: true })
    f.session.activate()
    expect(sent(f)).toEqual([])
    f.controller.setAudioRunning(true)
    expect(sent(f)).toEqual([No])
  })

  it('turns audio off when the context is suspended or closed, and on again when it runs', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.setAudioRunning(true)
    f.controller.updateSettings({ audio: true })
    f.controller.setAudioRunning(false)
    f.controller.setAudioRunning(true)
    f.controller.updateSettings({ audio: false })
    expect(sent(f)).toEqual([No, Yes, No, Yes])
  })

  it('follows the peer taking the audio permission away and back', async () => {
    const f = await connected({ audio: true })
    f.session.activate()
    f.controller.setAudioRunning(true)
    f.session.permissions.set({ ...f.session.permissions.get(), audio: false })
    f.session.permissions.set({ ...f.session.permissions.get(), audio: true })
    expect(sent(f)).toEqual([No, Yes, No])
  })

  it('stops asking while the session cannot play the audio, and asks again once it can', async () => {
    const f = await connected({ audio: true })
    f.session.activate()
    f.controller.setAudioRunning(true)
    f.session.audio.state.set('failed')
    f.session.audio.state.set('ok')
    f.session.audio.state.set('unsupported')
    expect(sent(f)).toEqual([No, Yes, No, Yes])
  })

  it('asks the next session for audio once it is active', async () => {
    const f = await connected({ audio: true })
    f.controller.setAudioRunning(true)
    f.session.activate()
    f.controller.connect()
    await settle()
    expect(loginOptionOf(f, 1)?.disableAudio).not.toBe(No)
    const next = f.sessions[1]
    if (!next) throw new Error('no second session')
    expect(sent(f, next)).toEqual([])
    next.activate()
    expect(sent(f, next)).toEqual([No])
  })
})

describe('RemoteController fps below the login one', () => {
  const DECODED_FRAMES = 13
  const TICKS = 8

  function sentFps(handshake: FakeHandshake): number[] {
    return handshake.sent.flatMap((m) =>
      m.union.case === 'misc' && m.union.value.union.case === 'option'
        ? [m.union.value.union.value.customFps].filter((fps) => fps !== 0)
        : [],
    )
  }

  async function streamed(clock: FakeClock, session: Session, link: Link) {
    link.handshake.events.push({ kind: 'state', state: { kind: 'active', peerInfo: PEER_INFO } })
    await vi.waitFor(() => {
      expect(session.state.get().kind).toBe('active')
    })
    for (let i = 0; i < DECODED_FRAMES; i++) {
      link.handshake.peerSends(msg({ case: 'videoFrame', value: vp9Frame(i === 0) }))
      await settle()
      clock.advance(1)
      link.decoders.last.output()
    }
    link.decoders.last.decodeQueueSize = BACKLOG
    clock.tick()
    link.decoders.last.decodeQueueSize = 0
    for (let i = 0; i < TICKS; i++) clock.tick()
  }

  interface Link {
    handshake: FakeHandshake
    decoders: FakeDecoders
  }

  it('is never raised by the session, before or after a reconnect', async () => {
    const f = fakeDeps()
    f.storage.setItem(SETTINGS_KEY, JSON.stringify({ v: 1, settings: { fps: 10 } }))
    const clock = new FakeClock()
    const links: Link[] = []
    const sessions: Session[] = []
    f.startSession.mockImplementation((_link, _params, _intent, maxFps) => {
      const link = { handshake: new FakeHandshake('relay'), decoders: new FakeDecoders() }
      links.push(link)
      const session = startSession(link.handshake, {
        maxFps,
        clock,
        createDecoder: link.decoders.create,
      })
      sessions.push(session)
      return session
    })
    const controller = new RemoteController(DEVICE, f.deps)
    controller.connect()
    await vi.waitFor(() => {
      expect(sessions).toHaveLength(1)
    })
    const [first, firstLink] = [sessions[0], links[0]]
    if (!first || !firstLink) throw new Error('no session started')
    await streamed(clock, first, firstLink)
    expect(sentFps(firstLink.handshake)).not.toEqual([])
    expect(Math.max(...sentFps(firstLink.handshake))).toBeLessThanOrEqual(10)

    firstLink.handshake.close({ by: 'link', reason: 'relay connection closed (code 1006)' })
    await vi.waitFor(
      () => {
        expect(sessions).toHaveLength(2)
      },
      { timeout: 3000 },
    )
    const [second, secondLink] = [sessions[1], links[1]]
    if (!second || !secondLink) throw new Error('no session restarted')
    await streamed(clock, second, secondLink)
    expect(sentFps(secondLink.handshake)).not.toEqual([])
    expect(Math.max(...sentFps(secondLink.handshake))).toBeLessThanOrEqual(10)
    controller.disconnect()
  })
})

describe('RemoteController codec', () => {
  it('asks an activated session for the stored codec: the login asked for VP9', async () => {
    const f = await connected({ codec: 'av1', quality: START_QUALITY })
    f.session.activate()
    expect(f.session.patches).toEqual([{ decoding: 'av1' }])
  })

  it('hands a codec change over to the session', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings({ codec: 'vp8' })
    expect(f.session.patches).toEqual([{ decoding: 'vp8' }])
  })

  it('puts the setting back to VP9 when the session gave up the codec', async () => {
    const f = await connected({ codec: 'av1', quality: START_QUALITY })
    f.session.activate()
    f.session.patches.length = 0
    f.session.codecs.set({
      ...f.session.codecs.get(),
      support: { vp9: 'ok', av1: 'failed', vp8: 'ok' },
      requested: 'vp9',
      failed: 'av1',
      failedAt: 5000,
    })
    expect(f.controller.settings.get().codec).toBe('vp9')
    expect(new RemoteController(DEVICE, f.deps).settings.get().codec).toBe('vp9')
    expect(f.session.patches).toEqual([{ decoding: 'vp9' }])
  })

  it('keeps a codec the user picked after the one that failed', async () => {
    const f = await connected({ codec: 'av1', quality: START_QUALITY })
    f.session.activate()
    f.controller.updateSettings({ codec: 'vp8' })
    f.session.codecs.set({ ...f.session.codecs.get(), failed: 'av1', failedAt: 5000 })
    expect(f.controller.settings.get().codec).toBe('vp8')
  })
})
