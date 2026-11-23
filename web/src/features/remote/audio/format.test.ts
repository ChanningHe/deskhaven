import { describe, expect, it } from 'vitest'

import { MAX_AUDIO_FRAME_BYTES, acceptsFrame, audioFormat } from './format'

describe('audioFormat', () => {
  it.each([
    [8000, 1],
    [12000, 2],
    [16000, 1],
    [24000, 2],
    [48000, 2],
  ])('takes %i Hz with %i channel(s)', (sampleRate, channels) => {
    expect(audioFormat({ sampleRate, channels })).toEqual({ sampleRate, channels })
  })

  it.each([
    ['a rate Opus does not run at', 44100, 2],
    ['no rate', 0, 2],
    ['a rate past the largest', 96000, 2],
    ['a rate one off', 48001, 2],
    ['no channels', 48000, 0],
    ['three channels', 48000, 3],
    ['a huge channel count', 48000, 0xffffffff],
  ])('refuses %s', (_, sampleRate, channels) => {
    expect(audioFormat({ sampleRate, channels })).toBeNull()
  })
})

describe('acceptsFrame', () => {
  it.each([
    ['one byte', 1, true],
    ['a full 10 ms frame', 1276, true],
    ['the largest', MAX_AUDIO_FRAME_BYTES, true],
    ['nothing', 0, false],
    ['one byte past the largest', MAX_AUDIO_FRAME_BYTES + 1, false],
    ['a megabyte', 1 << 20, false],
  ])('%s: %s', (_, size, accepted) => {
    expect(acceptsFrame(new Uint8Array(size))).toBe(accepted)
  })
})
