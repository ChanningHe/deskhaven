import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  DisplayInfoSchema,
  KeyboardMode,
  PeerInfoSchema,
  ResolutionSchema,
  SupportedResolutionsSchema,
  SwitchDisplaySchema,
} from '../proto/gen/message_pb.js'
import { peerView, withDisplays, withSwitch } from './peer-view'
import { MAX_RESOLUTIONS, offersResolution, parseResolutions } from './resolution'

const res = (width: number, height: number) => create(ResolutionSchema, { width, height })
const supported = (...r: [number, number][]) =>
  create(SupportedResolutionsSchema, { resolutions: r.map(([w, h]) => res(w, h)) })

describe('parseResolutions', () => {
  it('drops entries outside 1..16384, repeats, and sorts by area, largest first', () => {
    const list = [
      res(1024, 768),
      res(0, 600),
      res(800, -1),
      res(16385, 100),
      res(1920, 1080),
      res(1024, 768),
      res(16384, 1),
      res(1, 1),
    ]
    expect(parseResolutions(list)).toEqual([
      { width: 1920, height: 1080 },
      { width: 1024, height: 768 },
      { width: 16384, height: 1 },
      { width: 1, height: 1 },
    ])
  })

  it('rejects fractions', () => {
    expect(parseResolutions([{ width: 800.5, height: 600 }])).toEqual([])
  })

  it('keeps at most 64 entries, the largest ones', () => {
    const list = Array.from({ length: 100 }, (_, i) => res(100 + i, 100))
    const parsed = parseResolutions(list)
    expect(parsed).toHaveLength(MAX_RESOLUTIONS)
    expect(parsed[0]).toEqual({ width: 199, height: 100 })
    expect(parsed.at(-1)).toEqual({ width: 136, height: 100 })
  })

  it('reads a missing list as empty', () => {
    expect(parseResolutions(undefined)).toEqual([])
  })
})

function view() {
  const info = create(PeerInfoSchema, {
    version: '1.5.0',
    resolutions: supported([1280, 800], [1024, 768], [0, 0]),
    displays: [
      create(DisplayInfoSchema, { width: 1024, height: 768, originalResolution: res(1280, 800) }),
      create(DisplayInfoSchema, { x: 1024, width: 800, height: 600 }),
    ],
  })
  return peerView(info, KeyboardMode.Map)
}

describe('PeerView resolutions', () => {
  it('takes the list and the original resolution of the current display from PeerInfo', () => {
    expect(view()).toMatchObject({
      resolutions: [
        { width: 1280, height: 800 },
        { width: 1024, height: 768 },
      ],
      originalResolution: { width: 1280, height: 800 },
    })
  })

  it('has no original resolution when the peer sends none or a bad one', () => {
    const info = create(PeerInfoSchema, {
      displays: [
        create(DisplayInfoSchema, { width: 10, height: 10, originalResolution: res(0, 5) }),
      ],
    })
    expect(peerView(info, KeyboardMode.Map).originalResolution).toBeNull()
  })

  it('takes both from a later PeerInfo, keeping what it leaves out', () => {
    const update = create(PeerInfoSchema, {
      resolutions: supported([640, 480]),
      displays: [create(DisplayInfoSchema, { width: 640, height: 480 })],
    })
    expect(withDisplays(view(), update)).toMatchObject({
      resolutions: [{ width: 640, height: 480 }],
      originalResolution: { width: 1280, height: 800 },
    })
    const bare = create(PeerInfoSchema, {
      displays: [create(DisplayInfoSchema, { width: 640, height: 480 })],
    })
    expect(withDisplays(view(), bare).resolutions).toHaveLength(2)
  })

  it('takes both from SwitchDisplay, and empties them when it carries none', () => {
    const s = create(SwitchDisplaySchema, {
      display: 0,
      width: 800,
      height: 600,
      resolutions: supported([800, 600], [640, 480]),
      originalResolution: res(1280, 800),
    })
    expect(withSwitch(view(), s)).toMatchObject({
      display: { width: 800, height: 600 },
      resolutions: [{ width: 800 }, { width: 640 }],
      originalResolution: { width: 1280, height: 800 },
    })
    const other = withSwitch(
      view(),
      create(SwitchDisplaySchema, { display: 1, width: 800, height: 600 }),
    )
    expect(other.resolutions).toEqual([])
    expect(other.originalResolution).toBeNull()
  })
})

describe('offersResolution', () => {
  it('accepts a listed size and the original one only', () => {
    const v = view()
    expect(offersResolution(v, 1024, 768)).toBe(true)
    expect(offersResolution(v, 1280, 800)).toBe(true)
    expect(offersResolution(v, 1000, 700)).toBe(false)
    expect(offersResolution(v, 800, 600)).toBe(false)
  })
})
