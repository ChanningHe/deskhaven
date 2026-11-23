import { clone, create } from '@bufbuild/protobuf'

import {
  ImageQuality,
  OptionMessage_BoolOption,
  OptionMessageSchema,
  type OptionMessage,
} from '../proto/gen/message_pb.js'
import type { OptionPatch } from '../session/messages'
import { DEFAULT_FPS } from '../video/fps-governor'
import { sameQuality, START_QUALITY } from '../video/quality-governor'
import type { Quality, RemoteSettings } from './settings'

const PRESETS = {
  low: ImageQuality.Low,
  balanced: ImageQuality.Balanced,
  best: ImageQuality.Best,
} as const

// rustdesk 1.5.1 src/server/connection.rs:4929-4945: the peer reads custom_image_quality only
// while image_quality is NotSet, and as the value `q << 8` (video_qos.rs:435-446).
export function qualityFields(
  quality: Quality,
): Pick<OptionMessage, 'imageQuality' | 'customImageQuality'> {
  if (typeof quality === 'string') {
    return { imageQuality: PRESETS[quality], customImageQuality: 0 }
  }
  return { imageQuality: ImageQuality.NotSet, customImageQuality: quality.custom << 8 }
}

export function qualityOption(quality: Quality): OptionMessage {
  return create(OptionMessageSchema, qualityFields(quality))
}

export function loginSettings(settings: RemoteSettings): RemoteSettings {
  return { ...settings, quality: START_QUALITY, fps: Math.min(settings.fps, DEFAULT_FPS) }
}

// Without it the peer sends no CursorPosition (rustdesk 1.5.1 src/server/connection.rs:4968-4990).
export function loginOption(settings: RemoteSettings): OptionMessage {
  const told = loginSettings(settings)
  return create(OptionMessageSchema, {
    ...qualityFields(told.quality),
    customFps: told.fps,
    showRemoteCursor: OptionMessage_BoolOption.Yes,
  })
}

/** custom_fps and the image quality have one writer each during a session: their governors. */
export function ungoverned(option: OptionMessage): OptionMessage {
  const copy = clone(OptionMessageSchema, option)
  copy.customFps = 0
  copy.imageQuality = ImageQuality.NotSet
  copy.customImageQuality = 0
  return copy
}

export function optionPatch(prev: RemoteSettings, next: RemoteSettings): OptionPatch | null {
  const patch: OptionPatch = {}
  if (!sameQuality(prev.quality, next.quality)) patch.quality = next.quality
  if (prev.fps !== next.fps) patch.maxFps = next.fps
  return patch.quality !== undefined || patch.maxFps !== undefined ? patch : null
}
