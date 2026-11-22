import { describe, expect, it } from 'vitest'

import { decodeBase64 } from '../bytes'
import { compressBound, frameContentSize, inflate } from './zstd'

// Made with the zstd 1.5.7 CLI (`zstd -3`), which writes the content size as the peer does.
const CURSOR_2X2_ZST = decodeBase64('KLUv/SQQgQAA/wAA/wD/AP8AAP//AAAAAPgWgAQ=')
const CURSOR_2X2_RGBA = Uint8Array.of(
  0xff,
  0,
  0,
  0xff,
  0,
  0xff,
  0,
  0xff,
  0,
  0,
  0xff,
  0xff,
  0,
  0,
  0,
  0,
)
const TEXT_ZST = decodeBase64('KLUv/SQQgQAAaGVsbG8sIOWJqui0tOadv4Z3aNc=')
// `zstd --no-content-size` from a pipe: no frame content size in the header.
const TEXT_NO_SIZE_ZST = decodeBase64('KLUv/QRYgQAAaGVsbG8sIOWJqui0tOadv4Z3aNc=')

describe('frameContentSize', () => {
  it('reads the declared size of a single-segment frame', () => {
    expect(frameContentSize(CURSOR_2X2_ZST)).toBe(16)
    expect(frameContentSize(TEXT_ZST)).toBe(16)
  })

  it('is null without a declared size, a zstd magic or a whole header', () => {
    expect(frameContentSize(TEXT_NO_SIZE_ZST)).toBeNull()
    expect(frameContentSize(Uint8Array.of(1, 2, 3, 4, 5, 6))).toBeNull()
    expect(frameContentSize(CURSOR_2X2_ZST.subarray(0, 5))).toBeNull()
  })
})

describe('inflate', () => {
  it('decompresses a frame whose declared size fits', () => {
    expect(inflate(CURSOR_2X2_ZST, 16)).toEqual(CURSOR_2X2_RGBA)
    expect(new TextDecoder().decode(inflate(TEXT_ZST, 1024) ?? new Uint8Array())).toBe(
      'hello, 剪贴板',
    )
  })

  it('refuses a frame declaring more than the limit, before allocating it', () => {
    expect(inflate(CURSOR_2X2_ZST, 15)).toBeNull()
  })

  it('refuses a frame that does not declare its size', () => {
    expect(inflate(TEXT_NO_SIZE_ZST, 1024)).toBeNull()
  })

  it('refuses truncated data and a second frame', () => {
    expect(inflate(CURSOR_2X2_ZST.subarray(0, 12), 16)).toBeNull()
    const twoFrames = new Uint8Array([...CURSOR_2X2_ZST, ...CURSOR_2X2_ZST])
    expect(inflate(twoFrames, 32)).toBeNull()
  })
})

describe('compressBound', () => {
  // ZSTD_COMPRESSBOUND in zstd.h: srcSize + srcSize/256 + (128 KiB - srcSize) / 2048 below 128 KiB.
  it.each([
    [0, 64],
    [16, 79],
    [1024 * 1024, 1024 * 1024 + 4096],
  ])('of %i bytes is %i', (size, bound) => {
    expect(compressBound(size)).toBe(bound)
  })
})
