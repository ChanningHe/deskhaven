import { describe, expect, it } from 'vitest'

import { ImageQuality } from '../proto/gen/message_pb.js'
import { DEFAULT_SETTINGS, SETTINGS_KEY, type RemoteSettings } from '../settings/settings'
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

  it('logs in with the quality and the starting fps of the settings', async () => {
    const f = await connected({ quality: 'best', fps: 60 })
    expect(loginOptionOf(f)).toMatchObject({
      imageQuality: ImageQuality.Best,
      customImageQuality: 0,
      customFps: 30,
    })
  })

  it('logs in again with the settings as they are by then', async () => {
    const f = await connected()
    f.controller.updateSettings({ quality: { custom: 70 }, fps: 20 })
    f.controller.connect()
    await settle()
    expect(loginOptionOf(f, 1)).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: 70 << 8,
      customFps: 20,
    })
  })
})

describe('RemoteController settings in a session', () => {
  it('sends a quality change as one patch with the peer option and no fps', async () => {
    const f = await connected()
    f.session.activate()
    f.controller.updateSettings({ quality: 'best' })
    expect(f.session.patches).toHaveLength(1)
    const [patch] = f.session.patches
    expect(patch?.maxFps).toBeUndefined()
    expect(patch?.peer).toMatchObject({ imageQuality: ImageQuality.Best, customImageQuality: 0 })
  })

  it('hands an fps change over as a ceiling, which is all the session may be told', async () => {
    const f = await connected()
    f.session.activate()
    f.controller.updateSettings({ fps: 45 })
    expect(f.session.patches).toEqual([{ maxFps: 45 }])
  })

  it('puts the fps back to 30 with a preset quality', async () => {
    const f = await connected({ quality: { custom: 60 }, fps: 50 })
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateSettings({ quality: 'low' })
    expect(f.controller.settings.get()).toMatchObject({ quality: 'low', fps: 30 })
    expect(f.session.patches).toMatchObject([
      { maxFps: 30, peer: { imageQuality: ImageQuality.Low } },
    ])
  })

  it.each([
    { scale: 'original' },
    { clipboardSync: false },
    { showRemoteCursor: false },
    { audio: true },
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
    expect(f.session.patches).toMatchObject([{ maxFps: 60, peer: { customImageQuality: 70 << 8 } }])
    f.session.state.set({ kind: 'active' })
    expect(f.session.patches).toHaveLength(1)
  })

  it('sends nothing once the login already carried the setting', async () => {
    const f = await connected()
    f.controller.updateSettings({ quality: 'best' })
    f.controller.updateSettings({ quality: 'balanced' })
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
    f.session.end('gone')
    f.controller.updateSettings({ quality: 'low' })
    expect(f.session.patches).toEqual([])
  })

  it('only reaches the session of the current connect', async () => {
    const f = await connected()
    f.session.activate()
    f.controller.connect()
    await settle()
    f.controller.updateSettings({ quality: 'low' })
    expect(f.session.patches).toEqual([])
    expect(f.sessions[1]?.patches).toEqual([])
    f.sessions[1]?.activate()
    expect(f.sessions[1]?.patches).toMatchObject([{ peer: { imageQuality: ImageQuality.Low } }])
    expect(f.session.patches).toEqual([])
  })
})
