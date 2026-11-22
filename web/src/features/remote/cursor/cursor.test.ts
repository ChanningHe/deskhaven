import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

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
