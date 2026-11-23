import { describe, expect, it, vi } from 'vitest'

import type { CursorImages, CursorShape } from './cursor'
import { CssCursors, cssCursor, cursorFit, cursorScale, MAX_CSS_CURSOR } from './css-cursor'

function shape(id: string, size = 12, hot = 2): CursorShape {
  return {
    id,
    width: size,
    height: size + 6,
    hotX: hot,
    hotY: hot + 1,
    rgba: new Uint8ClampedArray(size * (size + 6) * 4),
  }
}

describe('cursorScale', () => {
  it.each([
    [800, 1280, 0.625],
    [790, 1280, 0.625],
    [640, 1280, 0.5],
    [1280, 1280, 1],
    [1300, 1280, 1],
    [2560, 1280, 2],
    [10, 1280, 0.125],
  ])('shows %i of %i peer pixels in steps of an eighth: %f', (shown, peer, scale) => {
    expect(cursorScale(shown, peer)).toBe(scale)
  })

  it('is null before either size is known', () => {
    expect(cursorScale(0, 1280)).toBeNull()
    expect(cursorScale(800, 0)).toBeNull()
  })
})

describe('cursorFit', () => {
  it('scales the image and its hotspot', () => {
    expect(cursorFit(shape('a'), 0.5)).toEqual({ width: 6, height: 9, hotX: 1, hotY: 2 })
    expect(cursorFit(shape('a'), 2)).toEqual({ width: 24, height: 36, hotX: 4, hotY: 6 })
  })

  it('keeps the hotspot inside the scaled image', () => {
    expect(cursorFit(shape('a', 3, 2), 0.125)).toEqual({ width: 1, height: 1, hotX: 0, hotY: 0 })
  })

  it(`gives up past ${String(MAX_CSS_CURSOR)} pixels a side`, () => {
    expect(cursorFit(shape('a', 64, 0), 1)).not.toBeNull()
    expect(cursorFit(shape('a', 58, 0), 2)).toEqual({ width: 116, height: 128, hotX: 0, hotY: 2 })
    expect(cursorFit(shape('a', 59, 0), 2)).toBeNull()
  })
})

describe('cssCursor', () => {
  it('points with the scaled hotspot and falls back to the default cursor', () => {
    expect(cssCursor('blob:x', { width: 6, height: 9, hotX: 1, hotY: 2 })).toBe(
      'url("blob:x") 1 2, auto',
    )
  })
})

class Images implements CursorImages {
  readonly kept = new Map<CursorShape, { scale: number; url: string }>()
  image(s: CursorShape, scale: number) {
    const kept = this.kept.get(s)
    return kept?.scale === scale ? kept.url : null
  }
  attach(s: CursorShape, scale: number, url: string) {
    this.kept.set(s, { scale, url })
    return true
  }
}

function deferred() {
  let resolve: (blob: Blob | null) => void = () => undefined
  const promise = new Promise<Blob | null>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

function setup() {
  let n = 0
  const createUrl = vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:${String(++n)}`)
  const images = new Images()
  const ready = vi.fn()
  const pending: { shape: CursorShape; resolve: (blob: Blob | null) => void }[] = []
  const render = vi.fn((s: CursorShape) => {
    const d = deferred()
    pending.push({ shape: s, resolve: d.resolve })
    return d.promise
  })
  const cursors = new CssCursors(images, ready, render)
  return { images, ready, pending, render, cursors, createUrl }
}

const settle = () => new Promise((r) => setTimeout(r, 0))

describe('CssCursors', () => {
  it('makes the image once, keeps it with the shape and tells when it is ready', async () => {
    const { ready, pending, render, cursors, images } = setup()
    const a = shape('a')
    expect(cursors.cursor(a, 0.5)).toBeNull()
    cursors.prepare(a, 0.5)
    cursors.prepare(a, 0.5)
    expect(render).toHaveBeenCalledTimes(1)
    expect(render).toHaveBeenCalledWith(a, { width: 6, height: 9, hotX: 1, hotY: 2 })
    pending[0]?.resolve(new Blob())
    await settle()
    expect(ready).toHaveBeenCalledTimes(1)
    expect(images.kept.get(a)).toEqual({ scale: 0.5, url: 'blob:1' })
    expect(cursors.cursor(a, 0.5)).toBe('url("blob:1") 1 2, auto')
    cursors.prepare(a, 0.5)
    expect(render).toHaveBeenCalledTimes(1)
  })

  it('drops a result that a later request overtook', async () => {
    const { ready, pending, cursors, images, createUrl } = setup()
    const a = shape('a')
    const b = shape('b')
    cursors.prepare(a, 1)
    cursors.prepare(b, 1)
    pending[1]?.resolve(new Blob())
    pending[0]?.resolve(new Blob())
    await settle()
    expect(ready).toHaveBeenCalledTimes(1)
    expect(images.kept.has(a)).toBe(false)
    expect(cursors.cursor(b, 1)).toBe('url("blob:1") 2 3, auto')
    expect(createUrl).toHaveBeenCalledTimes(1)
  })

  it('makes a new image for a new scale', async () => {
    const { pending, render, cursors } = setup()
    const a = shape('a')
    cursors.prepare(a, 1)
    pending[0]?.resolve(new Blob())
    await settle()
    cursors.prepare(a, 0.5)
    expect(render).toHaveBeenCalledTimes(2)
    expect(cursors.cursor(a, 0.5)).toBeNull()
  })

  it('makes nothing for a cursor too large, and does not retry one that failed', async () => {
    const { ready, pending, render, cursors } = setup()
    cursors.prepare(shape('big', 100), 2)
    expect(render).not.toHaveBeenCalled()
    const a = shape('a')
    cursors.prepare(a, 1)
    pending[0]?.resolve(null)
    await settle()
    cursors.prepare(a, 1)
    expect(render).toHaveBeenCalledTimes(1)
    expect(ready).not.toHaveBeenCalled()
    expect(cursors.cursor(a, 1)).toBeNull()
  })

  it('treats a render that throws as one that failed', async () => {
    const { ready, render, cursors } = setup()
    render.mockRejectedValueOnce(new Error('no canvas'))
    cursors.prepare(shape('a'), 1)
    await settle()
    expect(ready).not.toHaveBeenCalled()
  })
})
