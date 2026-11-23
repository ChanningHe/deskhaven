import { create } from '@bufbuild/protobuf'
import { beforeEach, describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import {
  AudioFormatSchema,
  AudioFrameSchema,
  MiscSchema,
  TestDelaySchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { MAX_AUDIO_FRAME_BYTES } from './format'
import {
  AudioPipeline,
  MAX_BACKLOG_S,
  MAX_DECODE_FAILURES,
  MAX_DECODE_QUEUE,
  MAX_DECODER_REBUILDS,
  REBUILD_WINDOW_MS,
  START_LEAD_S,
} from './pipeline'
import { FakeAudioDecoders, FakeOutput } from './testing'

function formatMessage(sampleRate: number, channels: number): Message {
  const value = create(AudioFormatSchema, { sampleRate, channels })
  return msg({
    case: 'misc',
    value: create(MiscSchema, { union: { case: 'audioFormat', value } }),
  })
}

function frameMessage(size = 100): Message {
  return msg({
    case: 'audioFrame',
    value: create(AudioFrameSchema, { data: new Uint8Array(size) }),
  })
}

const PACKET_FRAMES = 480 // 10 ms at 48 kHz
const PACKET_MS = 10
const NOW_MS = 10_000
const LEAD_MS = START_LEAD_S * 1000

let decoders: FakeAudioDecoders
let output: FakeOutput
let audio: AudioPipeline
let now: number

beforeEach(() => {
  decoders = new FakeAudioDecoders()
  output = new FakeOutput()
  now = 0
  audio = new AudioPipeline(decoders, () => now)
})

function later() {
  now += REBUILD_WINDOW_MS
}

function playing() {
  audio.play(output)
  audio.receive(formatMessage(48000, 2))
}

describe('AudioPipeline.receive', () => {
  it('takes the audio format and frames, and nothing else', () => {
    playing()
    expect(audio.receive(frameMessage())).toBe(true)
    expect(audio.receive(formatMessage(48000, 2))).toBe(true)
    expect(audio.receive(msg({ case: 'misc', value: create(MiscSchema) }))).toBe(false)
    expect(audio.receive(msg({ case: 'testDelay', value: create(TestDelaySchema) }))).toBe(false)
  })

  it('opens an Opus decoder at the told rate and channels on the first frame', () => {
    audio.play(output)
    audio.receive(formatMessage(24000, 1))
    expect(decoders.all).toHaveLength(0)
    audio.receive(frameMessage(7))
    expect(decoders.last.format).toEqual({ sampleRate: 24000, channels: 1 })
    expect(decoders.last.decoded.map((d) => d.length)).toEqual([7])
  })

  it('gives each packet a later timestamp', () => {
    playing()
    audio.receive(frameMessage())
    audio.receive(frameMessage())
    const [first = 0, second = 0] = decoders.last.timestamps
    expect(second).toBeGreaterThan(first)
  })

  it('drops frames before any format', () => {
    audio.play(output)
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(0)
  })

  it.each([
    ['empty', 0],
    ['oversized', MAX_AUDIO_FRAME_BYTES + 1],
  ])('drops an %s frame and decodes the next one', (_, size) => {
    playing()
    audio.receive(frameMessage(size))
    audio.receive(frameMessage(10))
    expect(decoders.last.decoded.map((d) => d.length)).toEqual([10])
  })

  it('stops on a format it cannot decode, and starts again on a good one', () => {
    playing()
    audio.receive(frameMessage())
    output.sounds.length = 0
    decoders.last.emit()
    audio.receive(formatMessage(44100, 2))
    expect(decoders.last.closed).toBe(true)
    expect(output.playing()).toEqual([])
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)

    audio.receive(formatMessage(16000, 2))
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(2)
    expect(decoders.last.format).toEqual({ sampleRate: 16000, channels: 2 })
  })

  it('keeps its decoder when told the same format again, and swaps it for another', () => {
    playing()
    audio.receive(frameMessage())
    audio.receive(formatMessage(48000, 2))
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)
    audio.receive(formatMessage(48000, 1))
    expect(decoders.all[0]?.closed).toBe(true)
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(2)
  })
})

describe('AudioPipeline output', () => {
  it('decodes nothing without an output, or while it is suspended', () => {
    audio.receive(formatMessage(48000, 2))
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(0)
    audio.play(output)
    output.running = false
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(0)
    output.running = true
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)
  })

  it('copies every channel as planar f32 and closes the AudioData', () => {
    playing()
    audio.receive(frameMessage())
    const data = decoders.last.emit(PACKET_FRAMES)
    expect(data.copied).toEqual([0, 1])
    expect(data.closed).toBe(true)
    const [sound] = output.sounds
    expect(sound?.buffer.channels.map((c) => c[0])).toEqual([1, 2])
  })

  it('schedules packets back to back, the first a little ahead of now', () => {
    playing()
    audio.receive(frameMessage())
    for (let i = 0; i < 3; i++) decoders.last.emit(PACKET_FRAMES)
    const start = NOW_MS + LEAD_MS
    expect(output.playing()).toEqual([
      [start, start + PACKET_MS],
      [start + PACKET_MS, start + 2 * PACKET_MS],
      [start + 2 * PACKET_MS, start + 3 * PACKET_MS],
    ])
  })

  it('stays continuous while the clock runs within the queue', () => {
    playing()
    audio.receive(frameMessage())
    decoders.last.emit(PACKET_FRAMES)
    output.currentTime += 0.03
    decoders.last.emit(PACKET_FRAMES)
    const start = NOW_MS + LEAD_MS
    expect(output.playing()[1]).toEqual([start + PACKET_MS, start + 2 * PACKET_MS])
  })

  it('starts over a little ahead of now after running dry', () => {
    playing()
    audio.receive(frameMessage())
    decoders.last.emit(PACKET_FRAMES)
    output.currentTime += 1
    decoders.last.emit(PACKET_FRAMES)
    const restart = NOW_MS + 1000 + LEAD_MS
    expect(output.playing()[1]).toEqual([restart, restart + PACKET_MS])
  })

  it('drops the oldest queued sound past the backlog limit and starts over from now', () => {
    playing()
    audio.receive(frameMessage())
    const queued = Math.ceil((MAX_BACKLOG_S * 1000 - LEAD_MS) / PACKET_MS) + 1
    for (let i = 0; i < queued; i++) decoders.last.emit(PACKET_FRAMES)
    expect(output.sounds.every((s) => !s.stopped)).toBe(true)
    decoders.last.emit(PACKET_FRAMES)
    expect(output.sounds.slice(0, queued).every((s) => s.stopped)).toBe(true)
    const restart = NOW_MS + LEAD_MS
    expect(output.playing()).toEqual([[restart, restart + PACKET_MS]])
    decoders.last.emit(PACKET_FRAMES)
    expect(output.playing()[1]).toEqual([restart + PACKET_MS, restart + 2 * PACKET_MS])
  })

  it('stops what it queued and closes the decoder when the output goes', () => {
    playing()
    audio.receive(frameMessage())
    decoders.last.emit(PACKET_FRAMES)
    audio.play(null)
    expect(decoders.last.closed).toBe(true)
    expect(output.playing()).toEqual([])
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)
  })

  it('keeps the format for the next output', () => {
    playing()
    audio.play(null)
    const next = new FakeOutput()
    audio.play(next)
    audio.receive(frameMessage())
    decoders.last.emit(PACKET_FRAMES)
    expect(next.sounds).toHaveLength(1)
  })

  it('closes but never plays what a replaced decoder still hands back', () => {
    playing()
    audio.receive(frameMessage())
    const old = decoders.last
    audio.receive(formatMessage(24000, 2))
    const data = old.emit(PACKET_FRAMES)
    expect(data.closed).toBe(true)
    expect(output.sounds).toEqual([])
  })

  it('ends for good on close', () => {
    playing()
    audio.receive(frameMessage())
    decoders.last.emit(PACKET_FRAMES)
    audio.close()
    expect(decoders.last.closed).toBe(true)
    expect(output.playing()).toEqual([])
    audio.play(new FakeOutput())
    audio.receive(formatMessage(48000, 2))
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)
  })
})

describe('AudioPipeline decode failures', () => {
  it(`gives up after ${String(MAX_DECODE_FAILURES)} failures in a row`, () => {
    playing()
    for (let i = 0; i < MAX_DECODE_FAILURES - 1; i++) {
      later()
      audio.receive(frameMessage())
      decoders.last.fail()
    }
    expect(audio.state.get()).toBe('ok')
    expect(decoders.all).toHaveLength(MAX_DECODE_FAILURES - 1)
    later()
    audio.receive(frameMessage())
    decoders.last.fail()
    expect(audio.state.get()).toBe('failed')
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(MAX_DECODE_FAILURES)
  })

  it('counts only failures in a row', () => {
    playing()
    for (let i = 0; i < MAX_DECODE_FAILURES * 2; i++) {
      later()
      audio.receive(frameMessage())
      if (i % 2 === 0) decoders.last.fail()
      else decoders.last.emit()
    }
    expect(audio.state.get()).toBe('ok')
  })

  it('counts a refused packet and a decoder that cannot be made', () => {
    playing()
    audio.receive(frameMessage())
    decoders.last.refuse = true
    audio.receive(frameMessage())
    decoders.refuse = true
    for (let i = 0; i < MAX_DECODE_FAILURES - 2; i++) {
      later()
      audio.receive(frameMessage())
    }
    expect(audio.state.get()).toBe('ok')
    later()
    audio.receive(frameMessage())
    expect(audio.state.get()).toBe('failed')
  })

  it('counts decoded sound the output cannot take, and still closes it', () => {
    playing()
    output.refuse = true
    audio.receive(frameMessage())
    const handed = Array.from({ length: MAX_DECODE_FAILURES }, () => decoders.last.emit())
    expect(handed.every((d) => d.closed)).toBe(true)
    expect(audio.state.get()).toBe('failed')
  })

  it('tries again from a clean count on the next output', () => {
    playing()
    for (let i = 0; i < MAX_DECODE_FAILURES; i++) {
      later()
      audio.receive(frameMessage())
      decoders.last.fail()
    }
    audio.play(null)
    expect(audio.state.get()).toBe('failed')
    audio.play(new FakeOutput())
    expect(audio.state.get()).toBe('ok')
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(MAX_DECODE_FAILURES + 1)
  })
})

describe('AudioPipeline limits', () => {
  it(`drops packets while ${String(MAX_DECODE_QUEUE)} wait in the decoder`, () => {
    playing()
    audio.receive(frameMessage(1))
    decoders.last.decodeQueueSize = MAX_DECODE_QUEUE
    audio.receive(frameMessage(2))
    decoders.last.decodeQueueSize = MAX_DECODE_QUEUE - 1
    audio.receive(frameMessage(3))
    expect(decoders.last.decoded.map((d) => d.length)).toEqual([1, 3])
  })

  it('fails once good and bad packets rebuild the decoder too often', () => {
    playing()
    for (let i = 0; i <= MAX_DECODER_REBUILDS; i++) {
      audio.receive(frameMessage())
      decoders.last.emit()
      decoders.last.fail()
    }
    expect(audio.state.get()).toBe('ok')
    audio.receive(frameMessage())
    expect(audio.state.get()).toBe('failed')
    expect(decoders.all).toHaveLength(MAX_DECODER_REBUILDS + 1)
  })

  it('fails once formats flipping back and forth rebuild the decoder too often', () => {
    playing()
    for (let i = 0; i <= MAX_DECODER_REBUILDS + 1; i++) {
      audio.receive(formatMessage(i % 2 ? 24000 : 48000, 2))
      audio.receive(frameMessage())
    }
    expect(audio.state.get()).toBe('failed')
    expect(decoders.all).toHaveLength(MAX_DECODER_REBUILDS + 1)
  })

  it('allows the same rebuilds again once the window has passed', () => {
    playing()
    for (let i = 0; i < 3 * MAX_DECODER_REBUILDS; i++) {
      if (i % (MAX_DECODER_REBUILDS + 1) === 0) later()
      audio.receive(frameMessage())
      decoders.last.emit()
      decoders.last.fail()
    }
    expect(audio.state.get()).toBe('ok')
  })
})

describe('AudioPipeline format support', () => {
  async function told(sampleRate: number, channels: number) {
    audio.receive(formatMessage(sampleRate, channels))
    await Promise.resolve()
    await Promise.resolve()
  }

  it('asks the browser about the format the peer told', async () => {
    audio.play(output)
    await told(24000, 1)
    expect(decoders.asked).toEqual([{ sampleRate: 24000, channels: 1 }])
  })

  it('stops at once on a format the browser cannot decode, and drops its frames', async () => {
    decoders.unsupported = [{ sampleRate: 16000, channels: 1 }]
    audio.play(output)
    await told(48000, 2)
    audio.receive(frameMessage())
    await told(16000, 1)
    expect(audio.state.get()).toBe('unsupported')
    expect(decoders.last.closed).toBe(true)
    audio.receive(frameMessage())
    expect(decoders.all).toHaveLength(1)
  })

  it('stays unsupported on another output, and recovers on a format it decodes', async () => {
    decoders.unsupported = [{ sampleRate: 16000, channels: 1 }]
    audio.play(output)
    await told(16000, 1)
    audio.play(new FakeOutput())
    expect(audio.state.get()).toBe('unsupported')
    await told(48000, 2)
    expect(audio.state.get()).toBe('ok')
  })

  it('ignores the answer about a format the peer already replaced', async () => {
    decoders.unsupported = [{ sampleRate: 16000, channels: 1 }]
    audio.play(output)
    audio.receive(formatMessage(16000, 1))
    await told(48000, 2)
    expect(audio.state.get()).toBe('ok')
  })

  it('takes a failed probe as not supported', async () => {
    decoders.probeThrows = true
    audio.play(output)
    await told(48000, 2)
    expect(audio.state.get()).toBe('unsupported')
  })
})
