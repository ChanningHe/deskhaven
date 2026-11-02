import { describe, expect, it, vi } from 'vitest'

import { readStorage, writeStorage } from './storage'

describe('storage', () => {
  it('round-trips values through localStorage', () => {
    expect(writeStorage('k', 'v')).toBe(true)
    expect(readStorage('k')).toBe('v')
  })

  it('degrades gracefully when storage throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota')
    })

    expect(readStorage('k')).toBeNull()
    expect(writeStorage('k', 'v')).toBe(false)
  })
})
