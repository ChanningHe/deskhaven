import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Credential } from '../handshake/handshake'
import { OptionMessage_BoolOption } from '../proto/gen/message_pb.js'
import type { OptionPatch } from '../session/messages'
import { NO_INTENT } from '../settings/protocol'
import { SETTINGS_KEY } from '../settings/settings'
import { RemoteController } from './controller'
import { fakeDeps } from './fake-deps'

const DEVICE = '123456789'
const { Yes, No } = OptionMessage_BoolOption
const CREDENTIAL: Credential = { salt: 'salt', salted: new Uint8Array(32) }

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

async function connected() {
  const f = fakeDeps()
  const controller = new RemoteController(DEVICE, f.deps)
  controller.connect()
  await settle()
  const session = f.sessions[0]
  if (!session) throw new Error('no session started')
  return { ...f, controller, session }
}

const aboutIntent = (patches: readonly OptionPatch[]) =>
  patches.filter((patch) => 'peer' in patch || 'viewOnly' in patch)

function peerOption(f: Awaited<ReturnType<typeof connected>>, call = 0) {
  return f.startSession.mock.calls[call]?.[1].loginOption
}

describe('RemoteController session intent', () => {
  it('starts off, tells its subscribers, and never reaches the storage', async () => {
    const f = await connected()
    expect(f.controller.intent.get()).toEqual(NO_INTENT)
    let notified = 0
    f.controller.intent.subscribe(() => notified++)
    f.controller.updateIntent({ viewOnly: true, lockAfterEnd: true })
    expect(f.controller.intent.get()).toEqual({ viewOnly: true, lockAfterEnd: true })
    expect(notified).toBe(1)
    expect(f.storage.getItem(SETTINGS_KEY)).toBeNull()
    f.controller.updateSettings({ fps: 20 })
    expect(f.storage.getItem(SETTINGS_KEY)).not.toMatch(/viewOnly|lockAfterEnd/)
  })

  it('is off again for a page that is opened anew', async () => {
    const f = await connected()
    f.controller.updateIntent({ viewOnly: true })
    f.controller.updateSettings({ fps: 20 })
    expect(new RemoteController(DEVICE, f.deps).intent.get()).toEqual(NO_INTENT)
  })

  it('logs in without it, and starts the session as it is', async () => {
    const f = await connected()
    expect(peerOption(f)).toMatchObject({ disableKeyboard: 0, lockAfterSessionEnd: 0 })
    expect(f.startSession.mock.calls[0]?.[2]).toEqual(NO_INTENT)
  })

  it('logs in with what it is by then', async () => {
    const f = await connected()
    f.controller.updateIntent({ viewOnly: true })
    f.controller.connect()
    await settle()
    expect(peerOption(f, 1)).toMatchObject({ disableKeyboard: Yes, disableClipboard: Yes })
    expect(f.startSession.mock.calls[1]?.[2]).toEqual({ viewOnly: true, lockAfterEnd: false })
  })

  it('tells nothing a session that is not active yet, and catches it up when it is', async () => {
    const f = await connected()
    f.controller.updateIntent({ lockAfterEnd: true })
    expect(f.session.patches).toEqual([])
    f.session.activate()
    const told = aboutIntent(f.session.patches)
    expect(told).toHaveLength(1)
    expect(told[0]?.peer).toMatchObject({ lockAfterSessionEnd: Yes })
  })

  it('does not tell it again what the login already carried', async () => {
    const f = await connected()
    f.controller.updateIntent({ viewOnly: true })
    f.controller.connect()
    await settle()
    f.sessions[1]?.activate()
    expect(aboutIntent(f.sessions[1]?.patches ?? [])).toEqual([])
  })

  it('hands a view only switch to the session with the option for the peer, changed fields only', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateIntent({ viewOnly: true })
    expect(f.session.patches).toHaveLength(1)
    expect(f.session.patches[0]).toMatchObject({
      viewOnly: true,
      peer: { disableKeyboard: Yes, disableClipboard: Yes, lockAfterSessionEnd: 0 },
    })
    f.controller.updateIntent({ viewOnly: false })
    expect(f.session.patches[1]).toMatchObject({
      viewOnly: false,
      peer: { disableKeyboard: No, disableClipboard: No, lockAfterSessionEnd: 0 },
    })
  })

  it('hands a lock switch over as the lock field alone', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateIntent({ lockAfterEnd: true })
    const [patch] = f.session.patches
    expect(patch).not.toHaveProperty('viewOnly')
    expect(patch?.peer).toMatchObject({
      lockAfterSessionEnd: Yes,
      disableKeyboard: 0,
      disableClipboard: 0,
    })
    f.controller.updateIntent({ lockAfterEnd: false })
    expect(f.session.patches[1]?.peer).toMatchObject({ lockAfterSessionEnd: No })
  })

  it('sends nothing for a switch that changes nothing', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateIntent({ viewOnly: false })
    f.controller.updateSettings({ scale: 'original' })
    expect(f.session.patches).toEqual([])
  })

  it('keeps the settings patch and the intent patch apart, each told once', async () => {
    const f = await connected()
    f.session.activate()
    f.session.patches.length = 0
    f.controller.updateIntent({ viewOnly: true })
    f.controller.updateSettings({ fps: 45 })
    expect(f.session.patches).toHaveLength(2)
    expect(f.session.patches[1]).toEqual({ maxFps: 45 })
  })
})

describe('RemoteController session intent across a reconnect', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('logs the new session in with the current switches', async () => {
    vi.useFakeTimers()
    const f = fakeDeps()
    const controller = new RemoteController(DEVICE, f.deps)
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    f.params(0).onHeard?.(Date.now())
    f.params(0).credentials?.keep(CREDENTIAL)
    const first = f.sessions[0]
    first?.activate()
    controller.updateIntent({ viewOnly: true, lockAfterEnd: true })

    first?.end({ by: 'link', reason: 'relay connection closed (code 1006)' })
    await vi.advanceTimersByTimeAsync(1000)

    expect(f.sessions).toHaveLength(2)
    expect(f.params(1).loginOption).toMatchObject({
      disableKeyboard: Yes,
      disableClipboard: Yes,
      lockAfterSessionEnd: 0,
    })
    expect(f.startSession.mock.calls[1]?.[2]).toEqual({ viewOnly: true, lockAfterEnd: true })
    f.sessions[1]?.activate()
    expect(aboutIntent(f.sessions[1]?.patches ?? [])).toEqual([])
    controller.updateIntent({ viewOnly: false })
    expect(aboutIntent(f.sessions[1]?.patches ?? [])[0]?.peer).toMatchObject({
      disableKeyboard: No,
      lockAfterSessionEnd: Yes,
    })
  })
})
