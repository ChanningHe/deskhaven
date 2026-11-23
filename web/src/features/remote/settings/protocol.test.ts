import { describe, expect, it } from 'vitest'

import { ImageQuality } from '../proto/gen/message_pb.js'
import { DEFAULT_FPS } from '../video/fps-governor'
import { loginOption, optionPatch, qualityFields } from './protocol'
import { DEFAULT_SETTINGS, type RemoteSettings } from './settings'

const with_ = (patch: Partial<RemoteSettings>): RemoteSettings => ({
  ...DEFAULT_SETTINGS,
  ...patch,
})

describe('qualityFields', () => {
  it.each([
    ['low', ImageQuality.Low],
    ['balanced', ImageQuality.Balanced],
    ['best', ImageQuality.Best],
  ] as const)('sends preset %s as image_quality alone', (quality, imageQuality) => {
    expect(qualityFields(quality)).toEqual({ imageQuality, customImageQuality: 0 })
  })

  it.each([10, 37, 100])('sends custom %i as q << 8 with image_quality unset', (q) => {
    expect(qualityFields({ custom: q })).toEqual({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: q << 8,
    })
  })
})

describe('loginOption', () => {
  it('carries the quality and the starting fps', () => {
    expect(loginOption(with_({ quality: 'best' }))).toMatchObject({
      imageQuality: ImageQuality.Best,
      customImageQuality: 0,
      customFps: DEFAULT_FPS,
    })
    expect(loginOption(with_({ quality: { custom: 80 } }))).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: 80 << 8,
    })
  })

  it('starts no faster than the governor does, and takes a lower user fps', () => {
    expect(loginOption(with_({ fps: 60 })).customFps).toBe(DEFAULT_FPS)
    expect(loginOption(with_({ fps: 12 })).customFps).toBe(12)
  })

  it('sets nothing else', () => {
    const { imageQuality, customImageQuality, customFps, ...rest } = loginOption(DEFAULT_SETTINGS)
    expect({ imageQuality, customImageQuality, customFps }).toEqual({
      imageQuality: ImageQuality.Balanced,
      customImageQuality: 0,
      customFps: DEFAULT_FPS,
    })
    expect(rest).toMatchObject({
      lockAfterSessionEnd: 0,
      showRemoteCursor: 0,
      disableAudio: 0,
      disableClipboard: 0,
      disableKeyboard: 0,
    })
    expect(rest.supportedDecoding).toBeUndefined()
  })
})

describe('optionPatch', () => {
  it('is null when nothing the peer or the session cares about changed', () => {
    expect(optionPatch(DEFAULT_SETTINGS, DEFAULT_SETTINGS)).toBeNull()
    expect(
      optionPatch(
        DEFAULT_SETTINGS,
        with_({ scale: 'original', showRemoteCursor: false, clipboardSync: false, audio: true }),
      ),
    ).toBeNull()
  })

  it('carries a changed quality and nothing else', () => {
    const patch = optionPatch(DEFAULT_SETTINGS, with_({ quality: 'best' }))
    expect(patch?.maxFps).toBeUndefined()
    expect(patch?.peer).toMatchObject({ imageQuality: ImageQuality.Best, customImageQuality: 0 })
    expect(patch?.peer?.customFps).toBe(0)
  })

  it('moves between a preset and a custom quality both ways', () => {
    const custom = optionPatch(DEFAULT_SETTINGS, with_({ quality: { custom: 60 } }))
    expect(custom?.peer).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: 60 << 8,
    })
    const back = optionPatch(with_({ quality: { custom: 60 } }), DEFAULT_SETTINGS)
    expect(back?.peer).toMatchObject({ imageQuality: ImageQuality.Balanced, customImageQuality: 0 })
    const other = optionPatch(
      with_({ quality: { custom: 60 } }),
      with_({ quality: { custom: 61 } }),
    )
    expect(other?.peer?.customImageQuality).toBe(61 << 8)
  })

  it('hands a changed fps over as a ceiling, never as custom_fps', () => {
    const patch = optionPatch(DEFAULT_SETTINGS, with_({ fps: 45 }))
    expect(patch).toEqual({ maxFps: 45 })
  })

  it('carries both when both changed', () => {
    const patch = optionPatch(DEFAULT_SETTINGS, with_({ quality: 'low', fps: 20 }))
    expect(patch?.maxFps).toBe(20)
    expect(patch?.peer?.imageQuality).toBe(ImageQuality.Low)
  })
})
