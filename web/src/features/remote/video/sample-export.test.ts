import { create } from '@bufbuild/protobuf'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { decodeBase64 } from '../bytes'
import { CLIENT_VERSION } from '../handshake/login'
import {
  EncodedVideoFramesSchema,
  VideoFrameSchema,
  type VideoFrame as VideoFrameMessage,
} from '../proto/gen/message_pb.js'
import {
  SAMPLE_FRAMES,
  SAMPLE_VERSION,
  SampleRecorder,
  REVOKE_AFTER_MS,
  downloadSample,
  encodeBase64,
  sampleFileName,
  type SampleCodec,
} from './sample-export'

const SOURCE = {
  peer: { platform: 'Windows', version: '1.5.1' },
  display: { width: 3840, height: 2160 },
}

let pts = 0n

function message(codec: SampleCodec, ...keys: boolean[]): VideoFrameMessage {
  return create(VideoFrameSchema, {
    union: {
      case: codec,
      value: create(EncodedVideoFramesSchema, {
        frames: keys.map((key) => {
          pts += 16n
          return { key, pts, data: Uint8Array.of(0, 0, 0, 1, Number(pts % 256n)) }
        }),
      }),
    },
  })
}

function setup(requested: SampleCodec = 'vp9s') {
  const target = { codec: requested }
  const recorder = new SampleRecorder(
    () => SOURCE,
    () => target.codec,
  )
  const progress: number[] = []
  recorder.progress.subscribe(() => {
    const state = recorder.progress.get()
    progress.push(state.kind === 'recording' ? state.frames : -1)
  })
  return { recorder, progress, target }
}

const deltas = (n: number) => Array<boolean>(n).fill(false)

describe('SampleRecorder', () => {
  it('keeps nothing until started', () => {
    const { recorder, progress } = setup()
    recorder.push(message('vp9s', true, false))
    expect(recorder.progress.get()).toEqual({ kind: 'idle' })
    expect(progress).toEqual([])
  })

  it(`starts at the next key frame and stops at exactly ${SAMPLE_FRAMES} frames`, async () => {
    const { recorder, progress } = setup()
    const done = recorder.start()
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    recorder.push(message('vp9s', false, false))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    const first = message('vp9s', false, true, false)
    recorder.push(first)
    for (let i = 0; i < 10; i++) recorder.push(message('vp9s', false, false, false))
    recorder.push(message('vp9s', false))

    const sample = await done
    if (!sample) throw new Error('no sample')
    expect(sample.frames).toHaveLength(SAMPLE_FRAMES)
    expect(sample.frames[0]?.key).toBe(true)
    expect(sample.frames.slice(1).every((f) => !f.key)).toBe(true)
    const [, key] = first.union.case === 'vp9s' ? first.union.value.frames : []
    expect(sample.frames[0]?.pts).toBe(Number(key?.pts))
    expect(recorder.progress.get()).toEqual({ kind: 'idle' })
    expect(progress.at(-2)).toBe(SAMPLE_FRAMES - 1)
    expect(progress.at(-1)).toBe(-1)
  })

  it('writes down the codec, the peer, the display and the versions', async () => {
    const { recorder } = setup('h265s')
    const done = recorder.start()
    recorder.push(message('h265s', true, ...deltas(SAMPLE_FRAMES - 1)))
    const sample = await done
    expect(sample).toMatchObject({
      version: SAMPLE_VERSION,
      clientVersion: CLIENT_VERSION,
      codec: 'h265s',
      complete: true,
      peer: SOURCE.peer,
      display: SOURCE.display,
    })
    expect(JSON.parse(JSON.stringify(sample))).toEqual(sample)
  })

  it('keeps the bytes exactly, through base64', async () => {
    const { recorder } = setup('h264s')
    const bytes = Uint8Array.from({ length: 256 }, (_, i) => i)
    const done = recorder.start()
    recorder.push(
      create(VideoFrameSchema, {
        union: {
          case: 'h264s',
          value: create(EncodedVideoFramesSchema, {
            frames: Array.from({ length: SAMPLE_FRAMES }, (_, i) => ({
              key: i === 0,
              pts: BigInt(i),
              data: bytes.subarray(i),
            })),
          }),
        },
      }),
    )
    const sample = await done
    sample?.frames.forEach((frame, i) => {
      expect(decodeBase64(frame.data)).toEqual(bytes.subarray(i))
    })
  })

  it('neither starts nor clears a recording on frames of a codec not asked for', () => {
    const { recorder } = setup('vp9s')
    void recorder.start()
    recorder.push(message('h264s', true))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    recorder.push(message('vp9s', true, false, false))
    recorder.push(message('h264s', true, false))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 3 })
  })

  it('waits for a key frame of the codec asked for, even if asked for after the start', async () => {
    const { recorder, target } = setup('vp9s')
    const done = recorder.start()
    target.codec = 'h264s'
    recorder.retarget()
    recorder.push(message('vp9s', true, false))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    recorder.push(message('h264s', true, ...deltas(SAMPLE_FRAMES - 1)))
    expect(await done).toMatchObject({ codec: 'h264s', complete: true })
  })

  it('gives back the H264 frames it has when the session falls back from H264', async () => {
    const { recorder, target } = setup('h264s')
    const done = recorder.start()
    recorder.push(message('h264s', true, false, false, false))
    target.codec = 'vp9s'
    recorder.push(message('vp9s', true))
    const sample = await done
    expect(sample).toMatchObject({ codec: 'h264s', complete: false })
    expect(sample?.frames).toHaveLength(4)
    expect(sample?.frames[0]?.key).toBe(true)
    expect(recorder.progress.get()).toEqual({ kind: 'idle' })
  })

  it('gives the partial H265 sample back as soon as it is told of the change', async () => {
    const { recorder, target } = setup('h265s')
    const done = recorder.start()
    recorder.push(message('h265s', true))
    target.codec = 'vp9s'
    recorder.retarget()
    expect(await done).toMatchObject({ codec: 'h265s', complete: false, frames: [{ key: true }] })
  })

  it('starts over on the new codec when the user moves off VP9 mid-recording', async () => {
    const { recorder, target } = setup('vp9s')
    const done = recorder.start()
    recorder.push(message('vp9s', true, false))
    target.codec = 'h264s'
    recorder.retarget()
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    recorder.push(message('h264s', true, ...deltas(SAMPLE_FRAMES - 1)))
    expect(await done).toMatchObject({ codec: 'h264s', complete: true })
  })

  it('passes over frames that are not encoded video', () => {
    const { recorder } = setup()
    void recorder.start()
    recorder.push(create(VideoFrameSchema, { union: { case: 'rgb', value: {} } }))
    recorder.push(create(VideoFrameSchema))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
  })

  it('gives nothing back once cancelled, and keeps nothing after', async () => {
    const { recorder } = setup()
    const done = recorder.start()
    recorder.push(message('vp9s', true, false))
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 2 })
    recorder.cancel()
    expect(await done).toBeNull()
    expect(recorder.progress.get()).toEqual({ kind: 'idle' })
    recorder.push(message('vp9s', true))
    expect(recorder.progress.get()).toEqual({ kind: 'idle' })
  })

  it('gives the earlier caller nothing when started again', async () => {
    const { recorder } = setup()
    const first = recorder.start()
    recorder.push(message('vp9s', true))
    const second = recorder.start()
    expect(await first).toBeNull()
    expect(recorder.progress.get()).toEqual({ kind: 'recording', frames: 0 })
    recorder.push(message('vp9s', true, ...deltas(SAMPLE_FRAMES - 1)))
    expect((await second)?.frames).toHaveLength(SAMPLE_FRAMES)
  })
})

describe('encodeBase64', () => {
  it('round-trips through decodeBase64', () => {
    const bytes = Uint8Array.from({ length: 70_000 }, (_, i) => (i * 7) % 256)
    expect(decodeBase64(encodeBase64(bytes))).toEqual(bytes)
    expect(encodeBase64(new Uint8Array())).toBe('')
  })
})

describe('sampleFileName', () => {
  it('names the codec and the local time', () => {
    expect(sampleFileName('h264s', new Date(2026, 10, 24, 9, 5, 7))).toBe(
      'video-sample-h264s-20261124-090507.json',
    )
  })
})

describe('downloadSample', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('keeps the file URL alive well past the click, for browsers that start the download late', async () => {
    vi.useFakeTimers()
    const revoke = vi.fn()
    Object.assign(URL, { createObjectURL: () => 'blob:sample', revokeObjectURL: revoke })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    const recorder = new SampleRecorder(
      () => SOURCE,
      () => 'vp9s',
    )
    const done = recorder.start()
    recorder.push(message('vp9s', true, ...deltas(SAMPLE_FRAMES - 1)))
    const sample = await done
    if (!sample) throw new Error('no sample')
    downloadSample(sample)
    expect(click).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(REVOKE_AFTER_MS - 1)
    expect(revoke).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(revoke).toHaveBeenCalledWith('blob:sample')
  })
})
