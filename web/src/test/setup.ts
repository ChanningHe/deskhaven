import '@testing-library/jest-dom/vitest'
import '@/i18n'

import { cleanup, configure } from '@testing-library/react'
import i18n from 'i18next'
import { afterEach, beforeEach, vi } from 'vitest'

import { stubMatchMedia } from './match-media'

// findBy* / waitFor give up after this. The default (1 s) is too tight on a busy
// shared CI runner; a passing test is not slowed down by a longer limit.
// vite.config.ts keeps testTimeout well above it.
const ASYNC_UTIL_TIMEOUT_MS = 5000
configure({ asyncUtilTimeout: ASYNC_UTIL_TIMEOUT_MS })

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
