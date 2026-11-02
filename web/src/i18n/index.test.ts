import { describe, expect, it } from 'vitest'

import i18n, { LANGUAGE_STORAGE_KEY, setLanguage } from './index'

describe('i18n runtime', () => {
  it('sets <html lang> at init and keeps it in sync on change', async () => {
    expect(document.documentElement.lang).toBe(i18n.resolvedLanguage)

    await setLanguage('en')
    expect(document.documentElement.lang).toBe('en')
    expect(window.localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en')

    await setLanguage('zh-CN')
    expect(document.documentElement.lang).toBe('zh-CN')
  })
})
