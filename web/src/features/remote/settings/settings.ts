import { DEFAULT_FPS } from '../video/fps-governor'

export type Preset = 'low' | 'balanced' | 'best'
export type Quality = Preset | { custom: number }
export type TransportSetting = 'auto' | 'relay'

export interface RemoteSettings {
  quality: Quality
  fps: number
  scale: 'fit' | 'original'
  showRemoteCursor: boolean
  clipboardSync: boolean
  audio: boolean
  transport: TransportSetting
}

export const SETTINGS_KEY = 'deskhaven.remote.settings'
export const SETTINGS_VERSION = 1
export const MIN_FPS = 5
export const MAX_FPS = 60
export const MIN_CUSTOM_QUALITY = 10
export const MAX_CUSTOM_QUALITY = 500

export const DEFAULT_SETTINGS: RemoteSettings = {
  quality: 'balanced',
  fps: DEFAULT_FPS,
  scale: 'fit',
  showRemoteCursor: true,
  clipboardSync: true,
  audio: false,
  transport: 'auto',
}

const PRESETS: readonly Preset[] = ['low', 'balanced', 'best']
export const TRANSPORTS: readonly TransportSetting[] = ['auto', 'relay']

/** Own data properties only: a stored `constructor` or `__proto__` key must not be read as a field. */
function field(source: unknown, name: string): unknown {
  if (typeof source !== 'object' || source === null || Array.isArray(source)) return undefined
  return Object.hasOwn(source, name) ? Reflect.get(source, name) : undefined
}

function inRange(value: unknown, min: number, max: number): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max
}

export function isQuality(value: unknown): value is Quality {
  if (PRESETS.some((preset) => preset === value)) return true
  return inRange(field(value, 'custom'), MIN_CUSTOM_QUALITY, MAX_CUSTOM_QUALITY)
}

function qualityFrom(value: unknown, fallback: Quality): Quality {
  if (typeof value === 'string') return isQuality(value) ? value : fallback
  const custom = field(value, 'custom')
  return inRange(custom, MIN_CUSTOM_QUALITY, MAX_CUSTOM_QUALITY) ? { custom } : fallback
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.find((option) => option === value) ?? fallback
}

function flag(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

/** Untrusted input: each field is checked on its own, and a bad one takes `fallback`'s value. */
export function settingsFrom(
  raw: unknown,
  fallback: RemoteSettings = DEFAULT_SETTINGS,
): RemoteSettings {
  const fps = field(raw, 'fps')
  return {
    quality: qualityFrom(field(raw, 'quality'), fallback.quality),
    fps: inRange(fps, MIN_FPS, MAX_FPS) ? fps : fallback.fps,
    scale: oneOf(field(raw, 'scale'), ['fit', 'original'], fallback.scale),
    showRemoteCursor: flag(field(raw, 'showRemoteCursor'), fallback.showRemoteCursor),
    clipboardSync: flag(field(raw, 'clipboardSync'), fallback.clipboardSync),
    audio: flag(field(raw, 'audio'), fallback.audio),
    transport: oneOf(field(raw, 'transport'), TRANSPORTS, fallback.transport),
  }
}

export function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage
  } catch {
    return null
  }
}

export function loadSettings(storage: Storage | null = browserStorage()): RemoteSettings {
  let text: string | null
  try {
    text = storage?.getItem(SETTINGS_KEY) ?? null
  } catch {
    return DEFAULT_SETTINGS
  }
  if (text === null) return DEFAULT_SETTINGS
  let stored: unknown
  try {
    stored = JSON.parse(text)
  } catch {
    return DEFAULT_SETTINGS
  }
  if (field(stored, 'v') !== SETTINGS_VERSION) return DEFAULT_SETTINGS
  return settingsFrom(field(stored, 'settings'))
}

export function saveSettings(
  settings: RemoteSettings,
  storage: Storage | null = browserStorage(),
): boolean {
  if (!storage) return false
  try {
    storage.setItem(SETTINGS_KEY, JSON.stringify({ v: SETTINGS_VERSION, settings }))
    return true
  } catch {
    return false
  }
}

/** Fields to change; an invalid one keeps its old value. */
export function withPatch(prev: RemoteSettings, patch: Partial<RemoteSettings>): RemoteSettings {
  return settingsFrom({ ...prev, ...patch }, prev)
}
