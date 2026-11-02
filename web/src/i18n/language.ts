// Supported UI languages and browser-language detection. Pure: no i18next,
// DOM or storage access, so it is trivially unit-testable.

export const SUPPORTED_LANGUAGES = ['zh-CN', 'en'] as const
export type Language = (typeof SUPPORTED_LANGUAGES)[number]

export const FALLBACK_LANGUAGE: Language = 'en'

// Native names: shown as-is regardless of the active UI language.
export const LANGUAGE_NAMES: Record<Language, string> = {
  'zh-CN': '简体中文',
  en: 'English',
}

export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (SUPPORTED_LANGUAGES as readonly string[]).includes(value)
}

/** Maps a BCP 47 tag (e.g. "zh-TW", "en-GB") to a supported language. */
export function matchLanguageTag(tag: string): Language | undefined {
  const primary = tag.toLowerCase().split('-')[0]
  // Only a Simplified Chinese catalog exists, so every Chinese variant
  // (zh-TW, zh-HK, zh-Hant, ...) maps to zh-CN until a Traditional one lands.
  if (primary === 'zh') return 'zh-CN'
  if (primary === 'en') return 'en'
  return undefined
}

/** Saved choice first, then the browser's preference order, then English. */
export function detectLanguage(
  stored: string | null,
  browserLanguages: readonly string[],
): Language {
  if (isLanguage(stored)) return stored
  for (const tag of browserLanguages) {
    const match = matchLanguageTag(tag)
    if (match) return match
  }
  return FALLBACK_LANGUAGE
}
