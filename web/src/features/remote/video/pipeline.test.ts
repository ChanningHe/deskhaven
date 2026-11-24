import { create } from '@bufbuild/protobuf'
import type { VideoCodec } from '../handshake/login'
import { describe, expect, it, vi } from 'vitest'

import { EncodedVideoFramesSchema, VideoFrameSchema } from '../proto/gen/message_pb.js'
import { encodedFrames, webCodecsString, type CodecEvents } from './codec'
import { FakeDecoders, fixtureKeyFrame, vp9Frame } from './fake-decoder'
import { MAX_DECODE_QUEUE, MAX_FAILURES, VideoPipeline } from './pipeline'

function setup() {
  const decoders = new FakeDecoders()
  const frames: VideoFrame[] = []
  const calls = { keyRequests: 0 }
  const onFatal = vi.fn()
  const pipeline = new VideoPipeline({
    createDecoder: decoders.create,
    onFrame: (frame) => frames.push(frame),
    requestKeyFrame: () => calls.keyRequests++,
    mayRefresh: () => true,
    onFatal,
  })
  return { decoders, frames, calls, onFatal, pipeline }
}

describe('codec selection', () => {
  it('decodes the fixture key frame as VP9 profile 0', () => {
    const encoded = encodedFrames(fixtureKeyFrame())
    expect(encoded?.codec).toBe('vp9')
    expect(webCodecsString('vp9')).toBe('vp09.00.10.08')
    expect(encoded?.frames[0]?.key).toBe(true)
  })

  it.each([
    ['av1s', 'av1', 'av01.0.04M.08'],
    ['vp8s', 'vp8', 'vp8'],
    ['h264s', 'h264', 'avc1.640033'],
    ['h265s', 'h265', 'hvc1.1.6.L153.B0'],
  ] as const)('maps %s to %s (%s)', (union, codec, string) => {
    const frame = create(VideoFrameSchema, {
      union: { case: union, value: create(EncodedVideoFramesSchema) },
    })
    expect(encodedFrames(frame)?.codec).toBe(codec)
    expect(webCodecsString(codec)).toBe(string)
  })

  it.each([
    ['rgb', create(VideoFrameSchema, { union: { case: 'rgb', value: {} } })],
    ['yuv', create(VideoFrameSchema, { union: { case: 'yuv', value: {} } })],
    ['nothing', create(VideoFrameSchema)],
  ])('decodes no %s', (_, frame) => {
    expect(encodedFrames(frame)).toBeNull()
  })
})

describe('VideoPipeline', () => {
  it.each([
    ['h264s', 'avc1.640033'],
    ['h265s', 'hvc1.1.6.L153.B0'],
  ] as const)(
    'decodes %s from its key frame on, with the key flags of the message',
    (union, string) => {
      const { decoders, calls, pipeline } = setup()
      const frames = (...keys: boolean[]) =>
        create(VideoFrameSchema, {
          union: {
            case: union,
            value: create(EncodedVideoFramesSchema, {
              frames: keys.map((key, i) => ({ key, data: Uint8Array.of(0, 0, 0, 1, i) })),
            }),
          },
        })
      pipeline.push(frames(false))
      expect(decoders.created).toHaveLength(0)
      expect(calls).toEqual({ keyRequests: 1 })
      pipeline.push(frames(false, true, false))
      expect(decoders.last.codec).toBe(string)
      expect(decoders.last.decoded.map((c) => [c.key, [...c.data]])).toEqual([
        [true, [0, 0, 0, 1, 1]],
        [false, [0, 0, 0, 1, 2]],
      ])
    },
  )

  it('decodes nothing before a key frame and asks for one once', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(vp9Frame(false))
    pipeline.push(vp9Frame(false))
    expect(decoders.created.flatMap((d) => d.decoded)).toEqual([])
    expect(calls).toEqual({ keyRequests: 1 })

    const key = fixtureKeyFrame()
    pipeline.push(key)
    expect(decoders.last.codec).toBe('vp09.00.10.08')
    expect(decoders.last.decoded).toHaveLength(1)
    expect(decoders.last.decoded[0]).toMatchObject({ key: true })
    expect(decoders.last.decoded[0]?.data).toHaveLength(544)
  })

  it('hands each decoded frame on', () => {
    const { decoders, frames, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    pipeline.push(vp9Frame(false))
    pipeline.push(vp9Frame(false))
    const decoder = decoders.last
    decoder.output(0)
    decoder.output(1)
    decoder.output(2)
    expect(calls).toEqual({ keyRequests: 0 })
    expect(frames.map((f) => f.timestamp)).toEqual(decoder.decoded.map((c) => c.timestamp))
  })

  it('on a decode error asks for a key frame and waits for it', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    pipeline.push(vp9Frame(false))
    const failed = decoders.last
    failed.fail()
    expect(calls).toEqual({ keyRequests: 1 })

    pipeline.push(vp9Frame(false))
    expect(calls).toEqual({ keyRequests: 1 })
    expect(failed.decoded).toHaveLength(2)

    pipeline.push(fixtureKeyFrame())
    expect(decoders.created).toHaveLength(2)
    expect(decoders.last.decoded).toHaveLength(1)
  })

  it('treats a refused chunk like a decode error', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    decoders.last.throwOnDecode = new Error('DataError')
    pipeline.push(vp9Frame(false))
    expect(decoders.last.closed).toBe(true)
    expect(calls).toEqual({ keyRequests: 1 })
  })

  it('under decoder backpressure drops delta frames until the next key frame', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    const decoder = decoders.last
    decoder.decodeQueueSize = MAX_DECODE_QUEUE
    pipeline.push(vp9Frame(false))
    expect(decoder.decoded).toHaveLength(1)
    expect(calls).toEqual({ keyRequests: 1 })

    decoder.decodeQueueSize = 0
    pipeline.push(vp9Frame(false))
    expect(decoder.decoded).toHaveLength(1)

    decoder.decodeQueueSize = MAX_DECODE_QUEUE
    pipeline.push(fixtureKeyFrame())
    expect(decoder.decoded).toHaveLength(2)
    expect(calls).toEqual({ keyRequests: 1 })
  })

  it('keeps decoding a backlog while a refresh is not allowed', () => {
    const decoders = new FakeDecoders()
    const calls = { keyRequests: 0, asked: 0 }
    let allow = false
    const pipeline = new VideoPipeline({
      createDecoder: decoders.create,
      onFrame: (frame) => {
        frame.close()
      },
      requestKeyFrame: () => calls.keyRequests++,
      mayRefresh: () => {
        calls.asked++
        return allow
      },
      onFatal: () => undefined,
    })
    pipeline.push(fixtureKeyFrame())
    const decoder = decoders.last
    pipeline.push(vp9Frame(false))
    expect(calls).toEqual({ keyRequests: 0, asked: 0 })

    decoder.decodeQueueSize = MAX_DECODE_QUEUE
    pipeline.push(vp9Frame(false))
    expect(decoder.decoded).toHaveLength(3)
    expect(calls).toEqual({ keyRequests: 0, asked: 1 })

    allow = true
    pipeline.push(vp9Frame(false))
    pipeline.push(vp9Frame(false))
    expect(decoder.decoded).toHaveLength(3)
    expect(calls).toEqual({ keyRequests: 1, asked: 2 })
  })

  it('reports how long the decoder spent on each frame, queued time left out', () => {
    const decoders = new FakeDecoders()
    let now = 0
    const decoded: [number, number][] = []
    const pipeline = new VideoPipeline({
      createDecoder: decoders.create,
      onFrame: (frame) => {
        frame.close()
      },
      requestKeyFrame: () => undefined,
      mayRefresh: () => true,
      onFatal: () => undefined,
      meter: {
        now: () => now,
        received: () => undefined,
        decoded: (latencyMs, serviceMs) => decoded.push([latencyMs, serviceMs]),
      },
    })
    pipeline.push(fixtureKeyFrame())
    now = 2
    pipeline.push(vp9Frame(false))
    now = 10
    decoders.last.output(0)
    now = 16
    decoders.last.output(1)
    now = 100
    pipeline.push(vp9Frame(false))
    now = 103
    decoders.last.output(2)
    expect(decoded).toEqual([
      [10, 10],
      [14, 6],
      [3, 3],
    ])
  })

  it('starts over at the next key frame after a reset, without asking for one', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    pipeline.push(vp9Frame(false))
    pipeline.reset()
    expect(decoders.last.closed).toBe(true)
    expect(calls).toEqual({ keyRequests: 0 })
    pipeline.push(vp9Frame(true))
    expect(decoders.created).toHaveLength(2)
    expect(calls.keyRequests).toBe(0)
  })

  it('opens a new decoder when the peer changes codec', () => {
    const { decoders, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    const av1Key = create(VideoFrameSchema, {
      union: {
        case: 'av1s',
        value: create(EncodedVideoFramesSchema, {
          frames: [{ key: true, data: Uint8Array.of(1) }],
        }),
      },
    })
    pipeline.push(av1Key)
    expect(decoders.created.map((d) => [d.codec, d.closed])).toEqual([
      ['vp09.00.10.08', true],
      ['av01.0.04M.08', false],
    ])
  })

  it('waits for a key frame of the new codec before opening its decoder', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    const av1Delta = create(VideoFrameSchema, {
      union: {
        case: 'av1s',
        value: create(EncodedVideoFramesSchema, {
          frames: [{ key: false, data: Uint8Array.of(1) }],
        }),
      },
    })
    pipeline.push(av1Delta)
    expect(decoders.created).toHaveLength(1)
    expect(calls).toEqual({ keyRequests: 1 })
  })

  it('skips encodings it cannot decode', () => {
    const { decoders, calls, pipeline } = setup()
    pipeline.push(create(VideoFrameSchema, { union: { case: 'rgb', value: {} } }))
    expect(decoders.created).toEqual([])
    expect(calls).toEqual({ keyRequests: 0 })
  })

  it(`gives up after ${MAX_FAILURES} failures in a row without a decoded frame`, () => {
    const { decoders, onFatal, pipeline } = setup()
    for (let i = 0; i < MAX_FAILURES; i++) {
      pipeline.push(fixtureKeyFrame())
      decoders.last.fail(new Error(`failure ${i}`))
    }
    expect(onFatal).toHaveBeenCalledOnce()
    expect(onFatal.mock.calls[0]?.[0]).toMatchObject({ message: `failure ${MAX_FAILURES - 1}` })
  })

  it('counts failures afresh for a new codec', () => {
    const { decoders, onFatal, pipeline } = setup()
    for (let i = 0; i < MAX_FAILURES - 1; i++) {
      pipeline.push(frameOf('av1s', true))
      decoders.last.fail()
    }
    for (let i = 0; i < MAX_FAILURES - 1; i++) {
      pipeline.push(fixtureKeyFrame())
      decoders.last.fail()
    }
    expect(onFatal).not.toHaveBeenCalled()
  })

  it('counts failures only in a row', () => {
    const { decoders, onFatal, pipeline } = setup()
    for (let i = 0; i < MAX_FAILURES; i++) {
      pipeline.push(fixtureKeyFrame())
      if (i === 1) decoders.last.output()
      decoders.last.fail()
    }
    expect(onFatal).not.toHaveBeenCalled()
  })

  it('closes frames that arrive after close, and the decoder', () => {
    const { decoders, frames, pipeline } = setup()
    pipeline.push(fixtureKeyFrame())
    pipeline.close()
    expect(decoders.last.closed).toBe(true)
    const late = decoders.last.output()
    expect(late.close).toHaveBeenCalledOnce()
    expect(frames).toEqual([])
    pipeline.push(fixtureKeyFrame())
    expect(decoders.created).toHaveLength(1)
  })
})

function frameOf(codec: 'av1s' | 'vp8s', key: boolean) {
  return create(VideoFrameSchema, {
    union: {
      case: codec,
      value: create(EncodedVideoFramesSchema, { frames: [{ key, data: Uint8Array.of(1) }] }),
    },
  })
}

class RecordedCodecs implements CodecEvents {
  readonly events: string[] = []
  refused = new Set<VideoCodec>()

  accepts(codec: VideoCodec): boolean {
    return !this.refused.has(codec)
  }

  received(codec: VideoCodec): void {
    this.events.push(`received ${codec}`)
  }

  decoded(codec: VideoCodec): void {
    this.events.push(`decoded ${codec}`)
  }

  failed(codec: VideoCodec): void {
    this.events.push(`failed ${codec}`)
  }
}

function watched() {
  const decoders = new FakeDecoders()
  const codecs = new RecordedCodecs()
  const calls = { keyRequests: 0, newDecoders: 0 }
  const onFatal = vi.fn()
  const pipeline = new VideoPipeline({
    createDecoder: decoders.create,
    onFrame: (frame) => {
      frame.close()
    },
    requestKeyFrame: () => calls.keyRequests++,
    mayRefresh: () => true,
    onFatal,
    codecs,
    onNewDecoder: () => calls.newDecoders++,
  })
  return { decoders, codecs, calls, onFatal, pipeline }
}

describe('VideoPipeline codecs', () => {
  it('reports the codec of what it receives, decodes and fails on', () => {
    const { decoders, codecs, pipeline } = watched()
    pipeline.push(fixtureKeyFrame())
    decoders.last.output()
    pipeline.push(frameOf('av1s', true))
    decoders.last.fail()
    pipeline.push(create(VideoFrameSchema, { union: { case: 'rgb', value: {} } }))
    expect(codecs.events).toEqual(['received vp9', 'decoded vp9', 'received av1', 'failed av1'])
  })

  it('reports a decode() that throws as a failure of its codec', () => {
    const { decoders, codecs, pipeline } = watched()
    pipeline.push(frameOf('vp8s', true))
    decoders.last.throwOnDecode = new Error('refused')
    pipeline.push(frameOf('vp8s', false))
    expect(codecs.events).toEqual(['received vp8', 'received vp8', 'failed vp8'])
  })

  it('takes a decoder that cannot be created as a failure, not an exception', () => {
    const { codecs, calls, onFatal, pipeline } = watched()
    const throwing = new VideoPipeline({
      createDecoder: () => {
        throw new TypeError('bad config')
      },
      onFrame: () => undefined,
      requestKeyFrame: () => calls.keyRequests++,
      mayRefresh: () => true,
      onFatal,
      codecs,
    })
    expect(() => {
      throwing.push(frameOf('av1s', true))
    }).not.toThrow()
    expect(codecs.events).toEqual(['received av1', 'failed av1'])
    expect(calls.keyRequests).toBe(1)
    expect(onFatal).not.toHaveBeenCalled()
    pipeline.close()
  })

  it('drops frames of a codec it is told not to decode, and asks for nothing', () => {
    const { decoders, codecs, calls, pipeline } = watched()
    codecs.refused.add('av1')
    pipeline.push(fixtureKeyFrame())
    pipeline.push(frameOf('av1s', true))
    pipeline.push(frameOf('av1s', false))
    expect(decoders.created.map((d) => d.codec)).toEqual(['vp09.00.10.08'])
    expect(decoders.last.closed).toBe(false)
    expect(calls.keyRequests).toBe(0)
    expect(pipeline.codec).toBe('av01.0.04M.08')
  })

  it('names the codec of the last frame received, decodable yet or not', () => {
    const { pipeline } = watched()
    expect(pipeline.codec).toBeNull()
    pipeline.push(fixtureKeyFrame())
    expect(pipeline.codec).toBe('vp09.00.10.08')
    pipeline.push(frameOf('vp8s', false))
    expect(pipeline.codec).toBe('vp8')
  })

  it('reconfigures at the key frame of a new codec, asking for one when a delta comes first', () => {
    const { decoders, calls, pipeline } = watched()
    pipeline.push(fixtureKeyFrame())
    pipeline.push(frameOf('vp8s', false))
    expect(calls).toEqual({ keyRequests: 1, newDecoders: 1 })
    pipeline.push(frameOf('vp8s', false))
    pipeline.push(frameOf('vp8s', true))
    pipeline.push(frameOf('vp8s', false))
    expect(calls).toEqual({ keyRequests: 1, newDecoders: 2 })
    expect(decoders.created.map((d) => [d.codec, d.closed])).toEqual([
      ['vp09.00.10.08', true],
      ['vp8', false],
    ])
    expect(decoders.last.decoded.map((c) => c.key)).toEqual([true, false])
  })

  it('tells of every new decoder, the one after a failure included', () => {
    const { decoders, calls, pipeline } = watched()
    pipeline.push(fixtureKeyFrame())
    pipeline.push(vp9Frame(false))
    decoders.last.fail()
    pipeline.push(fixtureKeyFrame())
    expect(calls.newDecoders).toBe(2)
  })
})
