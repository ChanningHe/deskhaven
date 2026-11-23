// Bundled only by web/e2e/codec/opus-decode.codec.ts; the app never imports it.
import { create } from '@bufbuild/protobuf'

import {
  AudioFormatSchema,
  AudioFrameSchema,
  MessageSchema,
  MiscSchema,
  type Message,
  type Misc,
} from '../proto/gen/message_pb.js'
import { webCodecsAudio } from './decoder'
import type { AudioFormat } from './format'
import { AudioPipeline, type AudioOutput, type AudioState } from './pipeline'
import { browserAudio, Speaker } from './speaker'

export interface SoundCheckResult {
  supported: boolean
  running: boolean
  packets: number
  largestPacket: number
  scheduled: number
  peak: number
  state: AudioState
}

const SECONDS = 0.2
const TONE_HZ = 440
const PACKET_MS = 10
const WAIT_MS = 5000
const POLL_MS = 20

function pause(ms = POLL_MS): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

// As the peer encodes: a packet per 10 ms (rustdesk 1.5.1 audio_service.rs:213).
async function encodeTone({ sampleRate, channels }: AudioFormat): Promise<Uint8Array[]> {
  const packets: Uint8Array[] = []
  const failures: Error[] = []
  const encoder = new AudioEncoder({
    output: (chunk) => {
      const data = new Uint8Array(chunk.byteLength)
      chunk.copyTo(data)
      packets.push(data)
    },
    error: (error) => {
      failures.push(error)
    },
  })
  encoder.configure({
    codec: 'opus',
    sampleRate,
    numberOfChannels: channels,
    opus: { frameDuration: PACKET_MS * 1000 },
  })
  const frames = sampleRate * SECONDS
  const pcm = new Float32Array(frames * channels)
  for (let i = 0; i < frames; i++) {
    const sample = 0.5 * Math.sin((2 * Math.PI * TONE_HZ * i) / sampleRate)
    for (let channel = 0; channel < channels; channel++) pcm[channel * frames + i] = sample
  }
  const data = new AudioData({
    format: 'f32-planar',
    sampleRate,
    numberOfChannels: channels,
    numberOfFrames: frames,
    timestamp: 0,
    data: pcm,
  })
  encoder.encode(data)
  data.close()
  await encoder.flush()
  encoder.close()
  const [failure] = failures
  if (failure) throw failure
  return packets
}

function misc(union: Misc['union']): Message {
  return create(MessageSchema, { union: { case: 'misc', value: create(MiscSchema, { union }) } })
}

function counting(output: AudioOutput, seen: { scheduled: number; peak: number }): AudioOutput {
  return {
    get running() {
      return output.running
    },
    get currentTime() {
      return output.currentTime
    },
    createBuffer: (channels, frames, sampleRate) =>
      output.createBuffer(channels, frames, sampleRate),
    schedule(buffer, when, onEnded) {
      seen.scheduled++
      for (const sample of buffer.getChannelData(0)) {
        seen.peak = Math.max(seen.peak, Math.abs(sample))
      }
      return output.schedule(buffer, when, onEnded)
    },
  }
}

export async function playTone(format: AudioFormat): Promise<SoundCheckResult> {
  const speaker = new Speaker(browserAudio)
  await speaker.probe()
  speaker.start()
  const output = speaker.view.get().output
  if (!output) throw new Error('no AudioContext')
  const packets = await encodeTone(format)
  const seen = { scheduled: 0, peak: 0 }
  const deadline = Date.now() + WAIT_MS
  while (!output.running && Date.now() < deadline) await pause()
  const pipeline = new AudioPipeline(webCodecsAudio)
  pipeline.play(counting(output, seen))
  const { sampleRate, channels } = format
  pipeline.receive(
    misc({ case: 'audioFormat', value: create(AudioFormatSchema, { sampleRate, channels }) }),
  )
  for (const data of packets) {
    pipeline.receive(
      create(MessageSchema, {
        union: { case: 'audioFrame', value: create(AudioFrameSchema, { data }) },
      }),
    )
    await pause(PACKET_MS)
  }
  while (seen.scheduled < packets.length && Date.now() < deadline) await pause()
  const result: SoundCheckResult = {
    supported: await webCodecsAudio.supports(format),
    running: output.running,
    packets: packets.length,
    largestPacket: Math.max(...packets.map((p) => p.length)),
    scheduled: seen.scheduled,
    peak: seen.peak,
    state: pipeline.state.get(),
  }
  pipeline.close()
  speaker.stop()
  return result
}
