import '@testing-library/jest-dom/vitest'
import '@/i18n'

import { cleanup } from '@testing-library/react'
import i18n from 'i18next'
import { afterEach, beforeEach, vi } from 'vitest'

import { stubMatchMedia } from './match-media'

beforeEach(async () => {
  stubMatchMedia(false)
  // The router's scroll restoration calls scrollTo, which jsdom does not implement.
  vi.stubGlobal('scrollTo', vi.fn())
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  cleanup()
  window.localStorage.clear()
  document.documentElement.className = ''
  vi.unstubAllGlobals()
})
