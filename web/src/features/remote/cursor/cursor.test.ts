import { create } from '@bufbuild/protobuf'
import { describe, expect, it, vi } from 'vitest'

import { decodeBase64 } from '../bytes'
import { CursorDataSchema, type CursorData } from '../proto/gen/message_pb.js'
import { CursorTracker, MAX_CACHED_SHAPES, decodeCursor } from './cursor'

// 2×2 RGBA (red, green, blue, transparent) compressed by the zstd 1.5.7 CLI.
const COLORS = decodeBase64('KLUv/SQQgQAA/wAA/wD/AP8AAP//AAAAAPgWgAQ=')
const RGBA = [0xff, 0, 0, 0xff, 0, 0xff, 0, 0xff, 0, 0, 0xff, 0xff, 0, 0, 0, 0]

function cursor(fields: Partial<CursorData> = {}): CursorData {
  return create(CursorDataSchema, {
    id: '7',
    hotx: 1,
    hoty: 0,
    width: 2,
    height: 2,
    colors: COLORS,
    ...fields,
  })
}

// Rules of rustdesk 1.5.0 src/client/io_loop.rs:2790-2828 (`decode_cursor_data`).
describe('decodeCursor', () => {
  it('inflates the colors to width × height RGBA with the hotspot', () => {
    const shape = decodeCursor(cursor())
    expect(shape).toMatchObject({ id: '7', width: 2, height: 2, hotX: 1, hotY: 0 })
    expect([...(shape?.rgba ?? [])]).toEqual(RGBA)
  })

  it.each([
    ['a zero width', { width: 0 }],
    ['a size over 512', { width: 513 }],
    ['a hotspot outside the image', { hotx: 2 }],
    ['a negative hotspot', { hoty: -1 }],
    ['colors of another size', { width: 1, hotx: 0 }],
    ['colors that are not zstd', { colors: Uint8Array.of(1, 2, 3) }],
  ])('rejects %s', (_, fields: Partial<CursorData>) => {
    expect(decodeCursor(cursor(fields))).toBeNull()
  })
})

describe('CursorTracker', () => {
  it('shows a shape on CursorData and again when CursorId names it', () => {
    const tracker = new CursorTracker()
    const shown = tracker.onData(cursor())
    expect(shown.shape?.id).toBe('7')
    tracker.onData(cursor({ id: '8', hotx: 0 }))
    expect(tracker.onId('7').shape?.id).toBe('7')
  })

  it('shows no shape for an unknown id or a shape that did not decode', () => {
    const tracker = new CursorTracker()
    tracker.onData(cursor())
    expect(tracker.onId('99').shape).toBeNull()
    tracker.onData(cursor())
    expect(tracker.onData(cursor({ id: '7', width: 0 })).shape).toBeNull()
    expect(tracker.onId('7').shape).toBeNull()
  })

  it('keeps the position apart from the shape', () => {
    const tracker = new CursorTracker()
    tracker.onData(cursor())
    expect(tracker.onPosition(30, -4)).toMatchObject({
      shape: { id: '7' },
      position: { x: 30, y: -4 },
    })
  })

  it(`keeps at most ${MAX_CACHED_SHAPES} shapes, dropping the least recently shown`, () => {
    const tracker = new CursorTracker()
    for (let i = 0; i <= MAX_CACHED_SHAPES; i++) {
      tracker.onData(cursor({ id: String(i) }))
      if (i === 1) tracker.onId('0')
    }
    expect(tracker.onId('0').shape?.id).toBe('0')
    expect(tracker.onId('1').shape).toBeNull()
  })
})

describe('CursorTracker images', () => {
  function tracked() {
    const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    const tracker = new CursorTracker()
    const shape = tracker.onData(cursor()).shape
    if (!shape) throw new Error('no shape')
    return { tracker, shape, revoke }
  }

  it('keeps one image per shape, at the scale it was made for', () => {
    const { tracker, shape, revoke } = tracked()
    expect(tracker.image(shape, 1)).toBeNull()
    expect(tracker.attach(shape, 1, 'blob:a')).toBe(true)
    expect(tracker.image(shape, 1)).toBe('blob:a')
    expect(tracker.image(shape, 0.5)).toBeNull()
    expect(tracker.attach(shape, 0.5, 'blob:b')).toBe(true)
    expect(revoke).toHaveBeenCalledWith('blob:a')
    expect(tracker.image(shape, 0.5)).toBe('blob:b')
    expect(tracker.image(shape, 1)).toBeNull()
  })

  it('keeps the image when CursorId shows the shape again', () => {
    const { tracker, shape, revoke } = tracked()
    tracker.attach(shape, 1, 'blob:a')
    tracker.onData(cursor({ id: '8', hotx: 0 }))
    expect(tracker.onId('7').shape).toBe(shape)
    expect(tracker.image(shape, 1)).toBe('blob:a')
    expect(revoke).not.toHaveBeenCalled()
  })

  it('revokes the image of a shape that new CursorData for its id replaces', () => {
    const { tracker, shape, revoke } = tracked()
    tracker.attach(shape, 1, 'blob:a')
    const next = tracker.onData(cursor()).shape
    expect(revoke).toHaveBeenCalledWith('blob:a')
    expect(next && tracker.image(next, 1)).toBeNull()
  })

  it('refuses and revokes an image for a shape it no longer keeps', () => {
    const { tracker, shape, revoke } = tracked()
    tracker.onData(cursor())
    expect(tracker.attach(shape, 1, 'blob:late')).toBe(false)
    expect(revoke).toHaveBeenCalledWith('blob:late')
  })

  it('revokes the image of the shape it drops for the cache limit', () => {
    const { tracker, shape, revoke } = tracked()
    tracker.attach(shape, 1, 'blob:first')
    for (let i = 0; i < MAX_CACHED_SHAPES; i++) tracker.onData(cursor({ id: `n${String(i)}` }))
    expect(revoke).toHaveBeenCalledWith('blob:first')
    expect(tracker.onId('7').shape).toBeNull()
  })

  it('revokes every image on close and refuses new ones', () => {
    const { tracker, shape, revoke } = tracked()
    const other = tracker.onData(cursor({ id: '8', hotx: 0 })).shape
    if (!other) throw new Error('no shape')
    tracker.attach(shape, 1, 'blob:a')
    tracker.attach(other, 1, 'blob:b')
    tracker.close()
    expect(revoke.mock.calls.map(([url]) => url).sort()).toEqual(['blob:a', 'blob:b'])
    expect(tracker.attach(shape, 1, 'blob:c')).toBe(false)
    expect(revoke).toHaveBeenLastCalledWith('blob:c')
  })
})
