import { create } from '@bufbuild/protobuf'

import { ImageQuality, OptionMessageSchema, type OptionMessage } from '../proto/gen/message_pb.js'
import type { OptionPatch } from '../session/messages'
import { DEFAULT_FPS } from '../video/fps-governor'
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

function sameQuality(a: Quality, b: Quality): boolean {
  if (typeof a === 'string' || typeof b === 'string') return a === b
  return a.custom === b.custom
}

export function loginOption(settings: RemoteSettings): OptionMessage {
  return create(OptionMessageSchema, {
    ...qualityFields(settings.quality),
    customFps: Math.min(settings.fps, DEFAULT_FPS),
  })
}

export function optionPatch(prev: RemoteSettings, next: RemoteSettings): OptionPatch | null {
  const patch: OptionPatch = {}
  if (!sameQuality(prev.quality, next.quality)) {
    patch.peer = create(OptionMessageSchema, qualityFields(next.quality))
  }
  if (prev.fps !== next.fps) patch.maxFps = next.fps
  return patch.peer || patch.maxFps !== undefined ? patch : null
}
