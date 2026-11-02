import { describe, expect, it } from 'vitest'

import { detectLanguage, matchLanguageTag } from './language'

describe('matchLanguageTag', () => {
  it('maps every Chinese variant to zh-CN (only catalog available)', () => {
    for (const tag of ['zh', 'zh-CN', 'zh-cn', 'zh-Hans', 'zh-TW', 'zh-HK', 'zh-Hant-TW']) {
      expect(matchLanguageTag(tag)).toBe('zh-CN')
    }
  })

  it('maps English variants to en and rejects others', () => {
    expect(matchLanguageTag('en-GB')).toBe('en')
    expect(matchLanguageTag('de-DE')).toBeUndefined()
    expect(matchLanguageTag('zhx')).toBeUndefined()
  })
})

describe('detectLanguage', () => {
  it('prefers a valid saved choice', () => {
    expect(detectLanguage('en', ['zh-CN'])).toBe('en')
  })

  it('ignores an invalid saved value', () => {
    expect(detectLanguage('fr', ['zh-TW'])).toBe('zh-CN')
  })

  it('walks the browser preference list in order', () => {
    expect(detectLanguage(null, ['de-DE', 'zh-HK', 'en-US'])).toBe('zh-CN')
    expect(detectLanguage(null, ['de-DE', 'en-US', 'zh-CN'])).toBe('en')
  })

  it('falls back to English', () => {
    expect(detectLanguage(null, ['de-DE', 'ja'])).toBe('en')
    expect(detectLanguage(null, [])).toBe('en')
  })
})
