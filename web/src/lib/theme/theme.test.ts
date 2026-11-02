import { describe, expect, it } from 'vitest'

import { applyTheme, isTheme, resolveTheme } from './theme'

describe('theme helpers', () => {
  it('resolves system against the OS preference', () => {
    expect(resolveTheme('system', true)).toBe('dark')
    expect(resolveTheme('system', false)).toBe('light')
    expect(resolveTheme('light', true)).toBe('light')
    expect(resolveTheme('dark', false)).toBe('dark')
  })

  it('accepts only known theme names', () => {
    expect(isTheme('dark')).toBe(true)
    expect(isTheme('blue')).toBe(false)
    expect(isTheme(null)).toBe(false)
  })

  it('toggles the dark class on the root element', () => {
    const root = document.createElement('html')

    applyTheme(root, 'dark')
    expect(root).toHaveClass('dark')

    applyTheme(root, 'light')
    expect(root).not.toHaveClass('dark')
  })
})
