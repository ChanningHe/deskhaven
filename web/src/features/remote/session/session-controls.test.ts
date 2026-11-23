import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { fixtureBytes } from '../__fixtures__/fixtures'
import { replayToKeyed } from '../__fixtures__/harness'
import { NO_MODIFIERS } from '../input/mouse'
import {
  DisplayInfoSchema,
  KeyboardMode,
  PeerInfoSchema,
  PermissionInfo_Permission,
  PermissionInfoSchema,
  ResolutionSchema,
  SupportedResolutionsSchema,
  SwitchDisplaySchema,
  WindowsSessionSchema,
  WindowsSessionsSchema,
} from '../proto/gen/message_pb.js'
import { startSession } from './session'
import { PEER_INFO, activeSession, misc, settle } from './testing'

const KEY_A = {
  kind: 'key' as const,
  down: true,
  key: 'a',
  code: 'KeyA',
  composing: false,
  altGraph: false,
  capsLock: false,
  numLock: false,
  modifiers: NO_MODIFIERS,
  time: 0,
}

describe('session keyboard mode', () => {
  it('switches mode, letting go of the keys held in the old one first', async () => {
    const r = await activeSession()
    r.session.input({ ...KEY_A, kind: 'key', code: 'ControlLeft', key: 'Control' })
    r.session.setKeyboardMode(KeyboardMode.Translate)
    r.session.input(KEY_A)
    expect(r.session.peer.get()?.keyboardMode).toBe(KeyboardMode.Translate)
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { mode: KeyboardMode.Map, down: true },
      { mode: KeyboardMode.Map, down: false },
      { mode: KeyboardMode.Translate, union: { case: 'seq', value: 'a' } },
    ])
  })

  it('ignores a mode the peer does not take', async () => {
    const android = create(PeerInfoSchema, { ...PEER_INFO, platform: 'Android', version: '1.4.5' })
    const r = await activeSession(android)
    r.session.setKeyboardMode(KeyboardMode.Map)
    expect(r.session.peer.get()?.keyboardMode).toBe(KeyboardMode.Translate)
  })
})

describe('session displays', () => {
  it('asks the peer to stream another display', async () => {
    const two = create(PeerInfoSchema, {
      ...PEER_INFO,
      displays: [
        create(DisplayInfoSchema, { width: 1280, height: 800 }),
        create(DisplayInfoSchema, { x: 1280, width: 1920, height: 1080 }),
      ],
    })
    const r = await activeSession(two)
    r.session.switchDisplay(1)
    r.session.switchDisplay(5)
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { union: { case: 'switchDisplay', value: { display: 1 } } },
      { union: { case: 'captureDisplays', value: { set: [1] } } },
    ])
  })
})

describe('session 2FA', () => {
  it('takes a code only while the peer asks for one', async () => {
    const r = await replayToKeyed('1.4.5')
    const session = startSession(r.handshake)
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    await settle()
    expect(session.state.get()).toEqual({ kind: 'need_password' })
    session.submit2fa('123456')
    expect(r.peerReceived().map((m) => m.union.case)).toEqual(['loginRequest'])
  })
})

function resolutionPeer(version: string) {
  return create(PeerInfoSchema, {
    ...PEER_INFO,
    version,
    resolutions: create(SupportedResolutionsSchema, {
      resolutions: [
        create(ResolutionSchema, { width: 1280, height: 800 }),
        create(ResolutionSchema, { width: 1024, height: 768 }),
      ],
    }),
    displays: [
      create(DisplayInfoSchema, { width: 1280, height: 800 }),
      create(DisplayInfoSchema, { x: 1280, width: 1920, height: 1080 }),
    ],
    currentDisplay: 1,
  })
}

describe('session changeResolution', () => {
  it('asks a 1.2.4 or newer peer to change the current display', async () => {
    const r = await activeSession(resolutionPeer('1.2.4'))
    r.session.changeResolution(1024, 768)
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      {
        union: {
          case: 'changeDisplayResolution',
          value: { display: 1, resolution: { width: 1024, height: 768 } },
        },
      },
    ])
  })

  it('uses the old message for an older peer', async () => {
    const r = await activeSession(resolutionPeer('1.2.3'))
    r.session.changeResolution(1024, 768)
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { union: { case: 'changeResolution', value: { width: 1024, height: 768 } } },
    ])
  })

  it('sends the original resolution of the display', async () => {
    const info = resolutionPeer('1.5.1')
    info.displays[1] = create(DisplayInfoSchema, {
      width: 1920,
      height: 1080,
      originalResolution: create(ResolutionSchema, { width: 2560, height: 1440 }),
    })
    const r = await activeSession(info)
    r.session.changeResolution(2560, 1440)
    expect(r.sentAfterLogin()).toHaveLength(1)
  })

  it('sends nothing for a size the peer did not list', async () => {
    const r = await activeSession(resolutionPeer('1.5.1'))
    r.session.changeResolution(1000, 700)
    r.session.changeResolution(Number.NaN, 768)
    expect(r.sentAfterLogin()).toEqual([])
  })

  it('sends nothing while viewing only', async () => {
    const r = await activeSession(resolutionPeer('1.5.1'), 'active', { viewOnly: true })
    r.session.changeResolution(1024, 768)
    expect(r.sentAfterLogin()).toEqual([])
  })

  it('sends nothing without keyboard permission', async () => {
    const r = await activeSession(resolutionPeer('1.5.1'))
    r.peerSends(
      misc({
        case: 'permissionInfo',
        value: create(PermissionInfoSchema, {
          permission: PermissionInfo_Permission.Keyboard,
          enabled: false,
        }),
      }),
    )
    await settle()
    r.session.changeResolution(1024, 768)
    expect(r.sentAfterLogin()).toEqual([])
  })

  it('sends nothing while the session is not active', async () => {
    const info = resolutionPeer('1.5.1')
    info.windowsSessions = create(WindowsSessionsSchema, {
      sessions: [
        create(WindowsSessionSchema, { sid: 1, name: 'Console' }),
        create(WindowsSessionSchema, { sid: 2, name: 'RDP' }),
      ],
      currentSid: 2,
    })
    const r = await activeSession(info, 'choose_session')
    r.session.changeResolution(1024, 768)
    expect(r.sentAfterLogin()).toEqual([])
  })
})

describe('session after the peer changes its resolution', () => {
  it('follows a SwitchDisplay: rectangle, list, and the pointer mapping', async () => {
    const r = await activeSession(resolutionPeer('1.5.1'))
    r.peerSends(
      misc({
        case: 'switchDisplay',
        value: create(SwitchDisplaySchema, {
          display: 1,
          x: 1280,
          width: 1024,
          height: 768,
          resolutions: create(SupportedResolutionsSchema, {
            resolutions: [
              create(ResolutionSchema, { width: 1920, height: 1080 }),
              create(ResolutionSchema, { width: 1024, height: 768 }),
            ],
          }),
          originalResolution: create(ResolutionSchema, { width: 1920, height: 1080 }),
        }),
      }),
    )
    await settle()
    expect(r.session.peer.get()).toMatchObject({
      display: { x: 1280, y: 0, width: 1024, height: 768 },
      resolutions: [{ width: 1920 }, { width: 1024 }],
      originalResolution: { width: 1920, height: 1080 },
    })
    r.session.input({
      kind: 'pointer',
      action: 'down',
      button: 'left',
      point: { x: 160, y: 100, width: 640, height: 400 },
      modifiers: NO_MODIFIERS,
    })
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([{ x: 1280 + 256, y: 192 }])
  })
})
