import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import { readStorage, writeStorage } from '@/lib/storage'

import { resources } from './resources'

export const SUPPORTED_LANGUAGES = ['zh-CN', 'en'] as const
export type Language = (typeof SUPPORTED_LANGUAGES)[number]

// Native names: shown as-is regardless of the active UI language.
export const LANGUAGE_NAMES: Record<Language, string> = {
  'zh-CN': '简体中文',
  en: 'English',
}

export const LANGUAGE_STORAGE_KEY = 'deskhaven.language'
const FALLBACK_LANGUAGE: Language = 'en'

export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (SUPPORTED_LANGUAGES as readonly string[]).includes(value)
}

function detectLanguage(): Language {
  const stored = readStorage(LANGUAGE_STORAGE_KEY)
  if (isLanguage(stored)) return stored
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh-CN' : FALLBACK_LANGUAGE
}

export async function setLanguage(language: Language): Promise<void> {
  writeStorage(LANGUAGE_STORAGE_KEY, language)
  await i18n.changeLanguage(language)
}

export function currentLanguage(): Language {
  return isLanguage(i18n.resolvedLanguage) ? i18n.resolvedLanguage : FALLBACK_LANGUAGE
}

i18n.on('languageChanged', (language) => {
  document.documentElement.lang = language
})

// Resources are bundled, so initialisation is synchronous: the first render
// already has translations and no Suspense boundary is needed.
void i18n.use(initReactI18next).init({
  resources,
  lng: detectLanguage(),
  fallbackLng: FALLBACK_LANGUAGE,
  supportedLngs: SUPPORTED_LANGUAGES,
  initAsync: false,
  interpolation: { escapeValue: false },
})

export default i18n
