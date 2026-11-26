import { describe, expect, it } from 'vitest'

import type { Quality } from '../settings/settings'
import {
  baseBitrate,
  bitrateModeled,
  formatKbps,
  formatRatio,
  ratioOf,
  targetKbps,
} from './bitrate'

describe('baseBitrate', () => {
  // Expected values worked by hand from the peer's base_bitrate: the preset whose pixel count is
  // nearest, scaled by the pixel ratio, then rounded.
  it.each([
    ['1920×1080 is its own preset', 1920, 1080, 2073],
    ['3840×2160 is its own preset', 3840, 2160, 5000],
    ['1280×720 is its own preset', 1280, 720, 1000],
    // 1 024 000 px: nearest is 1366×768 (1 049 088 px, 1100), 1100 × 1024000 / 1049088 = 1073.7
    ['1280×800 scales the nearest preset, 1366×768', 1280, 800, 1074],
    // 2 304 000 px: nearest is 2048×1080 (2 211 840 px, 2200), not 2560×1440; 2291.7
    ['1920×1200 scales 2048×1080', 1920, 1200, 2292],
    // 921 600 px sits between 1024×768 and 1366×768 but is 1280×720 exactly: no interpolation
    ['720×1280 takes the 1280×720 preset', 720, 1280, 1000],
  ])('%s', (_, width, height, kbps) => {
    expect(baseBitrate(width, height)).toBe(kbps)
  })
})

describe('targetKbps', () => {
  // base × ratio, truncated, as the peer's VPX encoder does.
  it.each<[string, Quality, number, number, number]>([
    ['1080p best: 2073 × 1.5 = 3109.5', 'best', 1920, 1080, 3109],
    ['1080p balanced: 2073 × 0.67 = 1388.9', 'balanced', 1920, 1080, 1388],
    ['1080p low: 2073 × 0.5 = 1036.5', 'low', 1920, 1080, 1036],
    ['4K best: 5000 × 1.5', 'best', 3840, 2160, 7500],
    ['4K balanced: 5000 × 0.67', 'balanced', 3840, 2160, 3350],
    ['720p best: 1000 × 1.5', 'best', 1280, 720, 1500],
    ['720p low: 1000 × 0.5', 'low', 1280, 720, 500],
    ['1280×800 custom 100: 1074 × 2', { custom: 100 }, 1280, 800, 2148],
    ['1280×800 custom 500: 1074 × 10', { custom: 500 }, 1280, 800, 10740],
    ['1080p custom 10: 2073 × 0.2 = 414.6', { custom: 10 }, 1920, 1080, 414],
  ])('%s', (_, quality, width, height, kbps) => {
    expect(targetKbps(quality, width, height)).toBe(kbps)
  })
})

describe('ratioOf', () => {
  it.each<[Quality, number]>([
    ['low', 0.5],
    ['balanced', 0.67],
    ['best', 1.5],
    [{ custom: 50 }, 1],
    [{ custom: 500 }, 10],
  ])('%o -> %o', (quality, ratio) => {
    expect(ratioOf(quality)).toBe(ratio)
  })
})

describe('formatKbps', () => {
  it.each([
    [3109, '~3.1 Mbps'],
    [10740, '~10.7 Mbps'],
    [1036, '~1 Mbps'],
    [1388, '~1.4 Mbps'],
    [619, '~620 kbps'],
    [414, '~410 kbps'],
    [996, '~1 Mbps'],
  ])('%o -> %o', (kbps, text) => {
    expect(formatKbps(kbps)).toBe(text)
  })
})

describe('formatRatio', () => {
  it.each([
    [1.5, '1.5×'],
    [0.67, '0.67×'],
    [0.66, '0.66×'],
    [10, '10×'],
  ])('%o -> %o', (ratio, text) => {
    expect(formatRatio(ratio)).toBe(text)
  })
})

describe('bitrateModeled', () => {
  it.each([
    [null, true],
    ['vp09.00.10.08', true],
    ['av01.0.04M.08', true],
    ['vp8', true],
    ['avc1.640033', false],
    ['hvc1.1.6.L153.B0', false],
  ])('%o -> %o', (codec, modeled) => {
    expect(bitrateModeled(codec)).toBe(modeled)
  })
})
