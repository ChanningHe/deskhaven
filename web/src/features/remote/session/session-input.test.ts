import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { fixtureBytes } from '../__fixtures__/fixtures'
import { msg, replayToKeyed } from '../__fixtures__/harness'
import { NO_MODIFIERS } from '../input/mouse'
import {
  ClipboardFormat,
  ClipboardSchema,
  KeyboardMode,
  MultiClipboardsSchema,
  PermissionInfo_Permission,
  PermissionInfoSchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { startSession } from './session'
import { activeSession, misc, settle } from './testing'

function permission(which: PermissionInfo_Permission, enabled: boolean): Message {
  return misc({
    case: 'permissionInfo',
    value: create(PermissionInfoSchema, { permission: which, enabled }),
  })
}

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
}

const CLICK = {
  kind: 'pointer' as const,
  action: 'down' as const,
  button: 'left' as const,
  point: { x: 160, y: 100, width: 640, height: 400 },
  modifiers: NO_MODIFIERS,
}

describe('session login states', () => {
  it('asks for the password when the peer wants one', async () => {
    const r = await replayToKeyed('1.4.5')
    const session = startSession(r.handshake)
    r.socket.receive(fixtureBytes('1.4.5', 'hbbr-enc-01-Hash'))
    await settle()
    expect(session.state.get()).toEqual({ kind: 'need_password' })
    expect(session.peer.get()).toBeNull()
    session.input(KEY_A)
    expect(r.peerReceived().map((m) => m.union.case)).toEqual(['loginRequest'])
  })
})

describe('session input', () => {
  it('sends a click at the peer position of the display', async () => {
    const r = await activeSession()
    r.session.input(CLICK)
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { mask: 1 | (1 << 3), x: 320, y: 200, modifiers: [] },
    ])
  })

  it('sends keys in Map mode with the Linux position code', async () => {
    const r = await activeSession()
    expect(r.session.peer.get()?.keyboardMode).toBe(KeyboardMode.Map)
    r.session.input(KEY_A)
    r.session.input({ ...KEY_A, down: false })
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { mode: KeyboardMode.Map, down: true, union: { case: 'chr', value: 38 } },
      { mode: KeyboardMode.Map, down: false, union: { case: 'chr', value: 38 } },
    ])
  })

  it('stops sending keyboard and mouse input while the peer has the keyboard off', async () => {
    const r = await activeSession()
    r.peerSends(permission(PermissionInfo_Permission.Keyboard, false))
    await settle()
    expect(r.session.permissions.get()).toEqual({ keyboard: false, clipboard: true })
    r.session.input(KEY_A)
    r.session.input(CLICK)
    r.session.input({ kind: 'wheel', deltaX: 0, deltaY: 10, modifiers: NO_MODIFIERS })
    expect(r.sentAfterLogin()).toEqual([])

    r.peerSends(permission(PermissionInfo_Permission.Keyboard, true))
    await settle()
    r.session.input(KEY_A)
    expect(r.sentAfterLogin()).toHaveLength(1)
  })
})

describe('session clipboard', () => {
  it('sends our text and shows the text the peer copies', async () => {
    const r = await activeSession()
    expect(r.session.clipboard('from the browser')).toBe(true)
    const sent = r.sentAfterLogin()[0]
    expect(sent?.union.case).toBe('clipboard')
    expect(new TextDecoder().decode((sent?.union.value as { content: Uint8Array }).content)).toBe(
      'from the browser',
    )

    const copied = create(ClipboardSchema, {
      format: ClipboardFormat.Text,
      content: new TextEncoder().encode('from the peer'),
    })
    r.peerSends(
      msg({
        case: 'multiClipboards',
        value: create(MultiClipboardsSchema, { clipboards: [copied] }),
      }),
    )
    await settle()
    expect(r.session.remoteClipboard.get()).toBe('from the peer')
  })

  it('neither sends nor takes clipboard text while the peer has the clipboard off', async () => {
    const r = await activeSession()
    r.peerSends(permission(PermissionInfo_Permission.Clipboard, false))
    await settle()
    expect(r.session.clipboard('secret')).toBe(false)
    const copied = create(ClipboardSchema, { content: new TextEncoder().encode('ignored') })
    r.peerSends(msg({ case: 'clipboard', value: copied }))
    await settle()
    expect(r.session.remoteClipboard.get()).toBeNull()
    expect(r.sentAfterLogin()).toEqual([])
  })
})
