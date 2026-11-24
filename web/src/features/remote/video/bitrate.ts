// Ported from RustDesk (AGPL-3.0), tag 1.5.1: libs/scrap/src/common/codec.rs:893-895 (BR_*),
// codec.rs:929-959 (base_bitrate), libs/scrap/src/common/vpxcodec.rs:325-327 (bitrate) and
// src/server/video_qos.rs:443 (a custom level q is the ratio q·2/100).
import { DECODER_PROBES } from '../handshake/login'
import type { Preset, Quality } from '../settings/settings'

export const PRESET_RATIO: Readonly<Record<Preset, number>> = {
  low: 0.5,
  balanced: 0.67,
  best: 1.5,
}

const RESOLUTION_PRESETS: readonly (readonly [number, number, number])[] = [
  [640, 480, 400],
  [800, 600, 500],
  [1024, 768, 800],
  [1280, 720, 1000],
  [1366, 768, 1100],
  [1440, 900, 1300],
  [1600, 900, 1500],
  [1920, 1080, 2073],
  [2048, 1080, 2200],
  [2560, 1440, 3000],
  [3440, 1440, 4000],
  [3840, 2160, 5000],
  [7680, 4320, 12000],
]

const KBPS_PER_MBPS = 1000

/** Base bitrate in kbps: the preset nearest in pixel count, scaled by the pixel ratio. */
export function baseBitrate(width: number, height: number): number {
  const pixels = width * height
  const off = ([w, h]: readonly [number, number, number]) => Math.abs(w * h - pixels)
  const [w, h, kbps] = RESOLUTION_PRESETS.reduce((a, b) => (off(b) < off(a) ? b : a))
  return Math.round(kbps * (pixels / (w * h)))
}

export function ratioOf(quality: Quality): number {
  return typeof quality === 'string' ? PRESET_RATIO[quality] : (quality.custom * 2) / 100
}

/** The peer's target bitrate in kbps for a VP9, AV1 or VP8 stream of this size. */
export function targetKbps(quality: Quality, width: number, height: number): number {
  return Math.trunc(baseBitrate(width, height) * ratioOf(quality))
}

const MODELED_CODECS: readonly string[] = Object.values(DECODER_PROBES)

/** Whether targetKbps describes a stream of this WebCodecs codec; null is the VP9 we ask for. */
export function bitrateModeled(codec: string | null): boolean {
  return codec === null || MODELED_CODECS.includes(codec)
}

export function formatKbps(kbps: number): string {
  const tens = Math.round(kbps / 10) * 10
  if (tens < KBPS_PER_MBPS) return `~${String(tens)} kbps`
  return `~${String(Number((kbps / KBPS_PER_MBPS).toFixed(1)))} Mbps`
}

export function formatRatio(ratio: number): string {
  return `${String(Number(ratio.toFixed(2)))}×`
}
