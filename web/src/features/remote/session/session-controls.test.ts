import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { fixtureBytes } from '../__fixtures__/fixtures'
import { replayToKeyed } from '../__fixtures__/harness'
import { NO_MODIFIERS } from '../input/mouse'
import {
  ControlKey,
  DisplayInfoSchema,
  KeyboardMode,
  PeerInfoSchema,
} from '../proto/gen/message_pb.js'
import { startSession } from './session'
import { PEER_INFO, activeSession, settle } from './testing'

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

describe('session Ctrl+Alt+Del', () => {
  it('sends Delete with Ctrl and Alt to a peer other than Windows', async () => {
    const r = await activeSession()
    r.session.ctrlAltDel()
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      {
        mode: KeyboardMode.Legacy,
        press: true,
        union: { case: 'controlKey', value: ControlKey.Delete },
        modifiers: [ControlKey.Alt, ControlKey.Control],
      },
    ])
  })

  it('sends the CtrlAltDel key to a Windows peer', async () => {
    const r = await activeSession(create(PeerInfoSchema, { ...PEER_INFO, platform: 'Windows' }))
    r.session.ctrlAltDel()
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { down: true, union: { case: 'controlKey', value: ControlKey.CtrlAltDel } },
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
