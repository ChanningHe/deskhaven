import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { NO_MODIFIERS, mouseMask } from '../input/mouse'
import {
  OptionMessage_BoolOption,
  OptionMessageSchema,
  PermissionInfo_Permission,
  PermissionInfoSchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { RESTART_REASON, SESSION_ACTIONS } from './messages'
import { RESTART_GRACE_MS } from './session'
import { FakeClock, fakeLinkSession, misc, settle } from './testing'

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

const MOVE = {
  kind: 'pointer' as const,
  action: 'move' as const,
  button: 'left' as const,
  point: { x: 160, y: 100, width: 640, height: 400 },
  modifiers: NO_MODIFIERS,
}

const WHEEL = { kind: 'wheel' as const, deltaX: 0, deltaY: 10, modifiers: NO_MODIFIERS }

function permission(which: PermissionInfo_Permission, enabled: boolean): Message {
  return misc({
    case: 'permissionInfo',
    value: create(PermissionInfoSchema, { permission: which, enabled }),
  })
}

const start = (options: Parameters<typeof fakeLinkSession>[2] = {}) =>
  fakeLinkSession('relay', new FakeClock(), options)

describe('view only', () => {
  it('drops keyboard, mouse, wheel and key actions: not one message leaves', async () => {
    const { session, handshake } = await start()
    session.setOptions({ viewOnly: true })
    session.input(KEY_A)
    session.input({ ...KEY_A, down: false })
    session.input(MOVE)
    session.input({ ...MOVE, action: 'down' })
    session.input(WHEEL)
    session.input({ kind: 'release' })
    for (const kind of SESSION_ACTIONS) session.action(kind)
    expect(handshake.sent).toEqual([])
  })

  it('sends input again once it is off', async () => {
    const { session, handshake } = await start()
    session.setOptions({ viewOnly: true })
    session.setOptions({ viewOnly: false })
    session.input(KEY_A)
    expect(handshake.sent).toHaveLength(1)
  })

  it('is on from the first frame when the session starts so', async () => {
    const { session, handshake } = await start({ viewOnly: true })
    session.input(KEY_A)
    session.action('win')
    expect(handshake.sent).toEqual([])
  })

  it('lets go of the keys held when it goes on, so none stays down on the peer', async () => {
    const { session, handshake } = await start()
    session.input(KEY_A)
    handshake.sent.length = 0
    session.setOptions({ viewOnly: true })
    expect(handshake.sent.map((m) => m.union.value)).toMatchObject([{ down: false }])
    session.setOptions({ viewOnly: true })
    expect(handshake.sent).toHaveLength(1)
  })

  it('lets go of the mouse buttons held when it goes on, where the pointer last was', async () => {
    const { session, handshake } = await start()
    session.input({ ...MOVE, action: 'down' })
    session.input({ ...MOVE, action: 'down', button: 'right' })
    session.input({ ...MOVE, point: { ...MOVE.point, x: 320, y: 200 } })
    handshake.sent.length = 0
    session.setOptions({ viewOnly: true })
    expect(handshake.sent.map((m) => m.union.value)).toMatchObject([
      { mask: mouseMask('up', 'left'), x: 640, y: 400 },
      { mask: mouseMask('up', 'right'), x: 640, y: 400 },
    ])
    session.setOptions({ viewOnly: false })
    session.setOptions({ viewOnly: true })
    expect(handshake.sent).toHaveLength(2)
  })

  it('has no button to let go of once they are up', async () => {
    const { session, handshake } = await start()
    session.input({ ...MOVE, action: 'down' })
    session.input({ ...MOVE, action: 'up' })
    handshake.sent.length = 0
    session.setOptions({ viewOnly: true })
    expect(handshake.sent).toEqual([])
  })

  it('keeps our clipboard from going out', async () => {
    const { session, handshake } = await start()
    session.setOptions({ viewOnly: true })
    expect(session.clipboard('secret')).toBe(false)
    expect(handshake.sent).toEqual([])
  })

  it('goes to the peer as the option the patch carries, and nothing besides', async () => {
    const { session, handshake } = await start()
    session.setOptions({
      viewOnly: true,
      peer: create(OptionMessageSchema, { disableKeyboard: OptionMessage_BoolOption.Yes }),
    })
    expect(handshake.sent.map((m) => m.union.value)).toMatchObject([
      { union: { case: 'option', value: { disableKeyboard: OptionMessage_BoolOption.Yes } } },
    ])
  })
})

describe('restart permission', () => {
  it('starts allowed: the peer reports only a no (rustdesk 1.5.0 connection.rs:687-689)', async () => {
    const { session } = await start()
    expect(session.permissions.get()).toEqual({
      keyboard: true,
      clipboard: true,
      restart: true,
      audio: true,
    })
  })

  it('follows what the peer reports', async () => {
    const { session, handshake } = await start()
    handshake.peerSends(permission(PermissionInfo_Permission.Restart, false))
    await settle()
    expect(session.permissions.get().restart).toBe(false)
    handshake.peerSends(permission(PermissionInfo_Permission.Restart, true))
    await settle()
    expect(session.permissions.get().restart).toBe(true)
  })
})

describe('restart', () => {
  it('asks the peer to restart', async () => {
    const { session, handshake } = await start()
    session.restart()
    expect(handshake.sent.map((m) => m.union.value)).toMatchObject([
      { union: { case: 'restartRemoteDevice', value: true } },
    ])
  })

  it('works while viewing only: it is not input', async () => {
    const { session, handshake } = await start({ viewOnly: true })
    session.restart()
    expect(handshake.sent).toHaveLength(1)
  })

  it('sends nothing without the peer’s permission', async () => {
    const { session, handshake } = await start()
    handshake.peerSends(permission(PermissionInfo_Permission.Restart, false))
    await settle()
    session.restart()
    expect(handshake.sent).toEqual([])
  })

  it('sends nothing before the session is active', async () => {
    const { session, handshake } = await start()
    session.close()
    handshake.sent.length = 0
    session.restart()
    expect(handshake.sent).toEqual([])
  })
})

describe('a link that drops after a restart', () => {
  const LINK = { by: 'link', reason: 'relay connection closed (code 1006)' } as const

  it('is the peer restarting, not a link to win back', async () => {
    const { session, handshake } = await start()
    session.restart()
    handshake.events.push({ kind: 'closed', cause: LINK })
    await settle()
    expect(session.state.get()).toEqual({
      kind: 'closed',
      cause: { by: 'peer', reason: RESTART_REASON },
    })
  })

  it('is the same when the peer says goodbye first', async () => {
    const { session, handshake } = await start()
    session.restart()
    handshake.peerSends(misc({ case: 'closeReason', value: 'Closed manually by the peer' }))
    await settle()
    expect(session.state.get()).toMatchObject({
      cause: { by: 'peer', reason: RESTART_REASON },
    })
  })

  it('stays a link loss when we never asked', async () => {
    const { session, handshake } = await start()
    handshake.events.push({ kind: 'closed', cause: LINK })
    await settle()
    expect(session.state.get()).toEqual({ kind: 'closed', cause: LINK })
  })

  it('stays a link loss when the peer ignored the request for long', async () => {
    const clock = new FakeClock()
    const { session, handshake } = await fakeLinkSession('relay', clock)
    session.restart()
    clock.advance(RESTART_GRACE_MS + 1)
    handshake.events.push({ kind: 'closed', cause: LINK })
    await settle()
    expect(session.state.get()).toEqual({ kind: 'closed', cause: LINK })
  })

  it('leaves our own hang-up ours', async () => {
    const { session } = await start()
    session.restart()
    session.close()
    expect(session.state.get()).toMatchObject({ cause: { by: 'user' } })
  })
})
