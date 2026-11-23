import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  ImageQuality,
  OptionMessage_BoolOption,
  OptionMessageSchema,
} from '../proto/gen/message_pb.js'
import { DEFAULT_FPS } from '../video/fps-governor'
import { START_LEVEL, START_QUALITY } from '../video/quality-governor'
import {
  NO_INTENT,
  intentPatch,
  loginOption,
  loginSettings,
  optionPatch,
  qualityFields,
  ungoverned,
} from './protocol'
import { DEFAULT_SETTINGS, type Quality, type RemoteSettings } from './settings'

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
  it("carries the quality governor's start, whatever the user's quality", () => {
    for (const quality of ['low', 'best', { custom: 80 }] as const) {
      expect(loginOption(with_({ quality }))).toMatchObject({
        imageQuality: ImageQuality.NotSet,
        customImageQuality: START_LEVEL << 8,
        customFps: DEFAULT_FPS,
      })
    }
  })

  it('starts no faster than the governor does, and takes a lower user fps', () => {
    expect(loginOption(with_({ fps: 60 })).customFps).toBe(DEFAULT_FPS)
    expect(loginOption(with_({ fps: 12 })).customFps).toBe(12)
  })

  it('tells what loginSettings says the peer was told', () => {
    const settings = with_({ quality: 'best', fps: 60 })
    expect(loginSettings(settings)).toEqual({ ...settings, quality: START_QUALITY, fps: 30 })
  })

  it('always asks for the remote cursor: without it the peer sends no CursorPosition', () => {
    for (const showRemoteCursor of [true, false]) {
      expect(loginOption(with_({ showRemoteCursor })).showRemoteCursor).toBe(
        OptionMessage_BoolOption.Yes,
      )
    }
  })

  it('sets nothing else', () => {
    const { imageQuality, customImageQuality, customFps, showRemoteCursor, ...rest } =
      loginOption(DEFAULT_SETTINGS)
    expect(showRemoteCursor).toBe(OptionMessage_BoolOption.Yes)
    expect({ imageQuality, customImageQuality, customFps }).toEqual({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: START_LEVEL << 8,
      customFps: DEFAULT_FPS,
    })
    expect(rest).toMatchObject({
      lockAfterSessionEnd: 0,
      disableAudio: 0,
      disableClipboard: 0,
      disableKeyboard: 0,
    })
    expect(rest.supportedDecoding).toBeUndefined()
  })
})

describe('ungoverned', () => {
  it('clears custom_fps and the image quality and keeps the rest', () => {
    const option = create(OptionMessageSchema, {
      imageQuality: ImageQuality.Best,
      customImageQuality: 70 << 8,
      customFps: 60,
      disableAudio: OptionMessage_BoolOption.Yes,
    })
    expect(ungoverned(option)).toMatchObject({
      imageQuality: ImageQuality.NotSet,
      customImageQuality: 0,
      customFps: 0,
      disableAudio: OptionMessage_BoolOption.Yes,
    })
    expect(option.customFps).toBe(60)
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

  it.each<[Quality, Quality]>([
    ['balanced', 'best'],
    ['balanced', { custom: 60 }],
    [{ custom: 60 }, 'balanced'],
    [{ custom: 60 }, { custom: 61 }],
  ])('hands a quality change %o → %o over as the cap alone', (from, to) => {
    expect(optionPatch(with_({ quality: from }), with_({ quality: to }))).toEqual({ quality: to })
  })

  it('hands a changed fps over as a ceiling, never as custom_fps', () => {
    const patch = optionPatch(DEFAULT_SETTINGS, with_({ fps: 45 }))
    expect(patch).toEqual({ maxFps: 45 })
  })

  it('carries both when both changed', () => {
    const patch = optionPatch(DEFAULT_SETTINGS, with_({ quality: 'low', fps: 20 }))
    expect(patch).toEqual({ quality: 'low', maxFps: 20 })
  })
})

const { Yes, No } = OptionMessage_BoolOption
const VIEW_ONLY = { ...NO_INTENT, viewOnly: true }
const LOCK = { ...NO_INTENT, lockAfterEnd: true }

describe('loginOption with the session intent', () => {
  it('asks for nothing extra by default', () => {
    expect(loginOption(DEFAULT_SETTINGS, NO_INTENT)).toMatchObject({
      disableKeyboard: 0,
      disableClipboard: 0,
      lockAfterSessionEnd: 0,
    })
  })

  it('view only: no keyboard, no clipboard, as the official client logs in', () => {
    expect(loginOption(DEFAULT_SETTINGS, VIEW_ONLY)).toMatchObject({
      disableKeyboard: Yes,
      disableClipboard: Yes,
      lockAfterSessionEnd: 0,
      showRemoteCursor: Yes,
    })
  })

  it('lock after the session ends', () => {
    expect(loginOption(DEFAULT_SETTINGS, LOCK).lockAfterSessionEnd).toBe(Yes)
  })

  it('sends no lock when viewing only: the peer would lock the screen of someone who never touched it', () => {
    const option = loginOption(DEFAULT_SETTINGS, { viewOnly: true, lockAfterEnd: true })
    expect(option.lockAfterSessionEnd).toBe(0)
  })
})

describe('intentPatch', () => {
  it('is null when nothing changed', () => {
    expect(intentPatch(NO_INTENT, NO_INTENT)).toBeNull()
    expect(intentPatch(VIEW_ONLY, VIEW_ONLY)).toBeNull()
  })

  it('turns view only on with the three fields the official client sets, and the local flag', () => {
    expect(intentPatch(NO_INTENT, VIEW_ONLY)).toMatchObject({
      viewOnly: true,
      peer: { disableKeyboard: Yes, disableClipboard: Yes, lockAfterSessionEnd: 0 },
    })
  })

  it('turns view only off again', () => {
    expect(intentPatch(VIEW_ONLY, NO_INTENT)).toMatchObject({
      viewOnly: false,
      peer: { disableKeyboard: No, disableClipboard: No, lockAfterSessionEnd: 0 },
    })
  })

  it('sends only the lock field when only the lock changed', () => {
    const on = intentPatch(NO_INTENT, LOCK)
    expect(on).not.toHaveProperty('viewOnly')
    expect(on?.peer).toMatchObject({
      lockAfterSessionEnd: Yes,
      disableKeyboard: 0,
      disableClipboard: 0,
    })
    expect(intentPatch(LOCK, NO_INTENT)?.peer).toMatchObject({ lockAfterSessionEnd: No })
  })

  it('turns the lock off while view only goes on, and back on after it', () => {
    const both = { viewOnly: true, lockAfterEnd: true }
    expect(intentPatch(LOCK, both)?.peer).toMatchObject({
      disableKeyboard: Yes,
      lockAfterSessionEnd: No,
    })
    expect(intentPatch(both, LOCK)?.peer).toMatchObject({
      disableKeyboard: No,
      lockAfterSessionEnd: Yes,
    })
  })

  it('tells the peer nothing about a lock that view only keeps from being sent', () => {
    expect(intentPatch(VIEW_ONLY, { viewOnly: true, lockAfterEnd: true })).toBeNull()
  })
})
