import { clone, create } from '@bufbuild/protobuf'

import {
  ImageQuality,
  OptionMessage_BoolOption,
  OptionMessageSchema,
  type OptionMessage,
} from '../proto/gen/message_pb.js'
import type { OptionPatch } from '../session/messages'
import { sameQuality, START_QUALITY } from '../video/quality-governor'
import { MAX_FPS, type Quality, type RemoteSettings } from './settings'

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
  return {
    ...settings,
    quality: START_QUALITY,
    fps: Math.min(settings.fps, MAX_FPS),
    audio: false,
    codec: 'vp9',
  }
}

export interface SessionIntent {
  viewOnly: boolean
  lockAfterEnd: boolean
}

export const NO_INTENT: SessionIntent = { viewOnly: false, lockAfterEnd: false }

const { Yes, No, NotSet } = OptionMessage_BoolOption

function disableAudio(settings: RemoteSettings): OptionMessage_BoolOption {
  return settings.audio ? No : Yes
}

// rustdesk 1.5.0 src/client.rs:3304-3324,3422-3449: view only disables keyboard and clipboard, no lock
function intentFields(intent: SessionIntent) {
  return {
    disableKeyboard: intent.viewOnly,
    disableClipboard: intent.viewOnly,
    lockAfterSessionEnd: intent.lockAfterEnd && !intent.viewOnly,
  }
}

// Without it the peer sends no CursorPosition (rustdesk 1.5.1 src/server/connection.rs:4968-4990).
export function loginOption(
  settings: RemoteSettings,
  intent: SessionIntent = NO_INTENT,
): OptionMessage {
  const told = loginSettings(settings)
  const { disableKeyboard, disableClipboard, lockAfterSessionEnd } = intentFields(intent)
  return create(OptionMessageSchema, {
    ...qualityFields(told.quality),
    customFps: told.fps,
    disableAudio: disableAudio(told),
    showRemoteCursor: Yes,
    disableKeyboard: disableKeyboard ? Yes : NotSet,
    disableClipboard: disableClipboard ? Yes : NotSet,
    lockAfterSessionEnd: lockAfterSessionEnd ? Yes : NotSet,
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

export function intentPatch(prev: SessionIntent, next: SessionIntent): OptionPatch | null {
  const before = intentFields(prev)
  const after = intentFields(next)
  const option = create(OptionMessageSchema, {})
  let changed = false
  for (const field of Object.keys(after) as (keyof typeof after)[]) {
    if (before[field] === after[field]) continue
    option[field] = after[field] ? Yes : No
    changed = true
  }
  const patch: OptionPatch = {}
  if (prev.viewOnly !== next.viewOnly) patch.viewOnly = next.viewOnly
  if (changed) patch.peer = option
  return patch.viewOnly !== undefined || patch.peer ? patch : null
}

export function optionPatch(prev: RemoteSettings, next: RemoteSettings): OptionPatch | null {
  const patch: OptionPatch = {}
  if (!sameQuality(prev.quality, next.quality)) patch.quality = next.quality
  if (prev.fps !== next.fps) patch.maxFps = next.fps
  if (prev.codec !== next.codec) patch.decoding = next.codec
  if (prev.audio !== next.audio) {
    patch.peer = create(OptionMessageSchema, { disableAudio: disableAudio(next) })
  }
  return Object.keys(patch).length > 0 ? patch : null
}
