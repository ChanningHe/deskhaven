import { describe, expect, it } from 'vitest'

import { stubMatchMedia } from '@/test/match-media'

import themeInitScript from '../../../public/theme-init.js?raw'
import { DARK_MEDIA_QUERY, THEME_STORAGE_KEY } from './theme'

function runThemeInit() {
  // Executes the exact file served before first paint (trusted repo content;
  // jsdom in vitest does not run <script> elements).
  // eslint-disable-next-line @typescript-eslint/no-implied-eval
  const run = new Function(themeInitScript) as () => void
  run()
}

describe('public/theme-init.js', () => {
  it('stays in sync with the theme module constants', () => {
    expect(themeInitScript).toContain(`'${THEME_STORAGE_KEY}'`)
    expect(themeInitScript).toContain(`'${DARK_MEDIA_QUERY}'`)
  })

  it('applies a persisted dark theme', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'dark')

    runThemeInit()

    expect(document.documentElement).toHaveClass('dark')
  })

  it('follows the system preference when nothing is stored', () => {
    stubMatchMedia(true)

    runThemeInit()

    expect(document.documentElement).toHaveClass('dark')
  })

  it('ignores invalid stored values', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'neon')

    runThemeInit()

    expect(document.documentElement).not.toHaveClass('dark')
  })
})
