import { create } from '@bufbuild/protobuf'
import { describe, expect, it, vi } from 'vitest'

import { EncodedVideoFramesSchema, VideoFrameSchema } from '../proto/gen/message_pb.js'
import { encodedFrames } from './codec'
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
    expect(encoded?.codec).toBe('vp09.00.10.08')
    expect(encoded?.frames[0]?.key).toBe(true)
  })

  it.each([
    ['av1s', 'av01.0.04M.08'],
    ['vp8s', 'vp8'],
  ] as const)('maps %s to %s', (union, codec) => {
    const frame = create(VideoFrameSchema, {
      union: { case: union, value: create(EncodedVideoFramesSchema) },
    })
    expect(encodedFrames(frame)?.codec).toBe(codec)
  })
})

describe('VideoPipeline', () => {
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
