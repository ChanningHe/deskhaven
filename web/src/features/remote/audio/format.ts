export interface AudioFormat {
  sampleRate: number
  channels: number
}

// rustdesk 1.5.1 src/server/audio_service.rs:478-491: Opus runs at these rates, mono or stereo.
const OPUS_RATES: readonly number[] = [8000, 12000, 16000, 24000, 48000]
const MAX_CHANNELS = 2

// 10 ms packets (audio_service.rs:213) are ≤ 1276 bytes (RFC 6716 §3.2.1); libopus writes ≤ 4000.
export const MAX_AUDIO_FRAME_BYTES = 4000

export function audioFormat({ sampleRate, channels }: AudioFormat): AudioFormat | null {
  if (!OPUS_RATES.includes(sampleRate)) return null
  if (!Number.isInteger(channels) || channels < 1 || channels > MAX_CHANNELS) return null
  return { sampleRate, channels }
}

export function acceptsFrame(data: Uint8Array): boolean {
  return data.length > 0 && data.length <= MAX_AUDIO_FRAME_BYTES
}
