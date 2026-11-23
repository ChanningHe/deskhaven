import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import { MAX_DECODE_FAILURES, REBUILD_WINDOW_MS } from '../audio/pipeline'
import { FakeAudioDecoders, FakeOutput } from '../audio/testing'
import {
  AudioFormatSchema,
  AudioFrameSchema,
  PermissionInfo_Permission,
  PermissionInfoSchema,
} from '../proto/gen/message_pb.js'
import { FakeClock, fakeLinkSession, misc, settle } from './testing'

const format = (sampleRate: number, channels: number) =>
  misc({ case: 'audioFormat', value: create(AudioFormatSchema, { sampleRate, channels }) })

const frame = (size = 100) =>
  msg({ case: 'audioFrame', value: create(AudioFrameSchema, { data: new Uint8Array(size) }) })

async function playing() {
  const audio = new FakeAudioDecoders()
  const clock = new FakeClock()
  const s = await fakeLinkSession('relay', clock, { audioDecoders: audio })
  const output = new FakeOutput()
  s.session.audio.play(output)
  return { ...s, audio, output, clock }
}

describe('Session audio', () => {
  it('hands the format and frames to the audio pipeline', async () => {
    const s = await playing()
    s.handshake.peerSends(format(48000, 2))
    s.handshake.peerSends(frame(64))
    await settle()
    expect(s.audio.last.format).toEqual({ sampleRate: 48000, channels: 2 })
    expect(s.audio.last.decoded.map((d) => d.length)).toEqual([64])
    s.audio.last.emit()
    expect(s.output.sounds).toHaveLength(1)
  })

  it('stays up through bad formats, oversized frames and failing decoders', async () => {
    const s = await playing()
    s.handshake.peerSends(format(44100, 9))
    s.handshake.peerSends(frame(1 << 20))
    s.handshake.peerSends(format(48000, 2))
    for (let i = 0; i < MAX_DECODE_FAILURES; i++) {
      s.clock.advance(REBUILD_WINDOW_MS)
      s.handshake.peerSends(frame())
      await settle()
      s.audio.last.fail()
    }
    s.handshake.peerSends(frame())
    await settle()
    expect(s.session.audio.state.get()).toBe('failed')
    expect(s.audio.all).toHaveLength(MAX_DECODE_FAILURES)
    expect(s.session.state.get().kind).toBe('active')
  })

  it('stops the sound when the session closes', async () => {
    const s = await playing()
    s.handshake.peerSends(format(48000, 2))
    s.handshake.peerSends(frame())
    await settle()
    s.audio.last.emit()
    s.session.close()
    expect(s.audio.last.closed).toBe(true)
    expect(s.output.playing()).toEqual([])
  })

  it('tracks the peer turning audio off and on', async () => {
    const s = await playing()
    const audioPermission = (enabled: boolean) =>
      misc({
        case: 'permissionInfo',
        value: create(PermissionInfoSchema, {
          permission: PermissionInfo_Permission.Audio,
          enabled,
        }),
      })
    expect(s.session.permissions.get().audio).toBe(true)
    s.handshake.peerSends(audioPermission(false))
    await settle()
    expect(s.session.permissions.get().audio).toBe(false)
    s.handshake.peerSends(audioPermission(true))
    await settle()
    expect(s.session.permissions.get().audio).toBe(true)
  })
})
