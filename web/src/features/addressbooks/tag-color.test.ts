import { describe, expect, it } from 'vitest'

import { argbToHex, DEFAULT_TAG_HEX, hexToArgb } from './tag-color'

describe('tag colours', () => {
  it('turns #rrggbb into an opaque unsigned ARGB value and back', () => {
    expect(hexToArgb('#ff0000')).toBe(0xffff0000)
    expect(hexToArgb('#9E9E9E')).toBe(0xff9e9e9e)
    expect(argbToHex(0xffff0000)).toBe('#ff0000')
    expect(argbToHex(0xff00000a)).toBe('#00000a')
  })

  it('drops the alpha of a colour another client wrote', () => {
    expect(argbToHex(0x80123456)).toBe('#123456')
  })

  it('refuses anything that is not a six digit hex colour', () => {
    for (const bad of ['', 'red', '#fff', '#12345', '#1234567', 'ff0000', '#gg0000']) {
      expect(hexToArgb(bad)).toBeUndefined()
    }
  })

  it('has a default that round-trips', () => {
    expect(argbToHex(hexToArgb(DEFAULT_TAG_HEX) ?? 0)).toBe(DEFAULT_TAG_HEX)
  })
})
