import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg, startFixture } from '../__fixtures__/harness'
import { decodeBase64 } from '../bytes'
import {
  CursorDataSchema,
  CursorPositionSchema,
  DisplayInfoSchema,
  KeyboardMode,
  PeerInfoSchema,
  SwitchDisplaySchema,
} from '../proto/gen/message_pb.js'
import { vp9Frame } from '../video/fake-decoder'
import { MAX_WAITING_FRAMES } from './frame-queue'
import { versionNumber } from './peer'
import { CLOSED_BY_USER, startSession } from './session'
import { PEER_INFO, activeSession, misc, settle } from './testing'

describe('versionNumber (hbb_common get_version_number)', () => {
  it.each([
    ['1.2.3', 1002030],
    ['1.1.10', 1001100],
    ['1.2.3-4', 1002034],
    ['', 0],
    ['x.y', 0],
  ])('%s is %i', (version, n) => {
    expect(versionNumber(version)).toBe(n)
  })
})

describe('session login', () => {
  it('passes the login states through until active', async () => {
    const started = startFixture('1.4.5')
    const session = startSession(started.handshake)
    expect(session.state.get()).toEqual({ kind: 'connecting' })
    started.socket.receive(new Uint8Array([0x1a, 0])) // not a SignedId: the handshake fails
    await settle()
    expect(session.state.get()).toMatchObject({ kind: 'closed' })
  })

  it('becomes active with the peer info, its display and a keyboard mode', async () => {
    const { session } = await activeSession()
    expect(session.peer.get()).toMatchObject({
      info: { platform: 'Linux', version: '1.4.5' },
      current: 0,
      display: { x: 0, y: 0, width: 1280, height: 800 },
      keyboardMode: KeyboardMode.Map,
    })
  })
})

describe('session video', () => {
  it('decodes the captured VP9 key frame and hands it on', async () => {
    const r = await activeSession()
    r.peerSendsCapturedFrame()
    await settle()
    const decoder = r.decoders.last
    expect(decoder.codec).toBe('vp09.00.10.08')
    expect(decoder.decoded.map((c) => [c.key, c.data.length])).toEqual([[true, 544]])
    expect(r.sentAfterLogin().filter((m) => m.union.case === 'misc')).toEqual([])

    const out = decoder.output()
    const frames = r.session.frames[Symbol.asyncIterator]()
    expect((await frames.next()).value).toBe(out)
  })

  it('asks the peer for a key frame when the stream starts without one', async () => {
    const r = await activeSession()
    r.peerSends(msg({ case: 'videoFrame', value: vp9Frame(false) }))
    await settle()
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { union: { case: 'refreshVideoDisplay', value: 0 } },
    ])
  })

  it('keeps only the newest frames for a renderer that falls behind', async () => {
    const r = await activeSession()
    r.peerSends(msg({ case: 'videoFrame', value: vp9Frame(true) }))
    r.peerSends(msg({ case: 'videoFrame', value: vp9Frame(false) }))
    r.peerSends(msg({ case: 'videoFrame', value: vp9Frame(false) }))
    await settle()
    const [oldest, older, newest] = [0, 1, 2].map((i) => r.decoders.last.output(i))
    expect(MAX_WAITING_FRAMES).toBe(2)
    expect(oldest?.close).toHaveBeenCalledOnce()
    const frames = r.session.frames[Symbol.asyncIterator]()
    expect((await frames.next()).value).toBe(older)
    expect((await frames.next()).value).toBe(newest)
    expect(older?.close).not.toHaveBeenCalled()
  })

  it('starts over at a key frame on a display switch and follows the new rectangle', async () => {
    const r = await activeSession()
    r.peerSends(msg({ case: 'videoFrame', value: vp9Frame(true) }))
    r.peerSends(
      misc({
        case: 'switchDisplay',
        value: create(SwitchDisplaySchema, {
          display: 1,
          x: 1280,
          y: 0,
          width: 1920,
          height: 1080,
        }),
      }),
    )
    await settle()
    expect(r.decoders.last.closed).toBe(true)
    expect(r.session.peer.get()).toMatchObject({
      current: 1,
      display: { x: 1280, y: 0, width: 1920, height: 1080 },
    })
  })

  it('takes the displays of a later PeerInfo', async () => {
    const r = await activeSession()
    const update = create(PeerInfoSchema, {
      displays: [create(DisplayInfoSchema, { x: 0, y: 0, width: 2560, height: 1440 })],
    })
    r.peerSends(msg({ case: 'peerInfo', value: update }))
    await settle()
    expect(r.session.peer.get()).toMatchObject({
      info: { platform: PEER_INFO.platform },
      displays: [{ width: 2560 }],
      display: { width: 2560, height: 1440 },
    })
  })
})

describe('session end', () => {
  it('closes with the reason the peer gives', async () => {
    const r = await activeSession()
    r.peerSends(misc({ case: 'closeReason', value: 'Closed manually by the peer' }))
    await settle()
    expect(r.session.state.get()).toEqual({ kind: 'closed', reason: 'Closed manually by the peer' })
    expect(r.socket.closedWith).toBe(1000)
  })

  it('tells the peer before hanging up, and ends the frame stream', async () => {
    const r = await activeSession()
    const frames = r.session.frames[Symbol.asyncIterator]()
    r.session.close()
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      { union: { case: 'closeReason', value: '' } },
    ])
    expect(r.session.state.get()).toEqual({ kind: 'closed', reason: CLOSED_BY_USER })
    expect(await frames.next()).toEqual({ done: true, value: undefined })
    r.session.input({ kind: 'text', text: 'late' })
    expect(r.sentAfterLogin()).toHaveLength(1)
  })

  it('closes when the link drops', async () => {
    const r = await activeSession()
    r.socket.drop(1006)
    await settle()
    const state = r.session.state.get()
    expect(state.kind === 'closed' && state.reason).toContain('1006')
  })
})

describe('session cursor', () => {
  it('follows the peer cursor shape and position', async () => {
    const r = await activeSession()
    const colors = decodeBase64('KLUv/SQQgQAA/wAA/wD/AP8AAP//AAAAAPgWgAQ=')
    const shape = create(CursorDataSchema, {
      id: '5',
      width: 2,
      height: 2,
      hotx: 0,
      hoty: 1,
      colors,
    })
    r.peerSends(msg({ case: 'cursorData', value: shape }))
    r.peerSends(
      msg({ case: 'cursorPosition', value: create(CursorPositionSchema, { x: 40, y: 50 }) }),
    )
    await settle()
    expect(r.session.cursor.get()).toMatchObject({
      shape: { id: '5', width: 2, height: 2, hotX: 0, hotY: 1 },
      position: { x: 40, y: 50 },
    })
    r.peerSends(msg({ case: 'cursorId', value: '6' }))
    await settle()
    expect(r.session.cursor.get().shape).toBeNull()
  })
})
