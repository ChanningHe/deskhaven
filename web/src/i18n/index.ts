import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import { readStorage, writeStorage } from '@/lib/storage'

import { detectLanguage, FALLBACK_LANGUAGE, isLanguage, SUPPORTED_LANGUAGES } from './language'
import type { Language } from './language'
import { resources } from './resources'

export { isLanguage, LANGUAGE_NAMES, SUPPORTED_LANGUAGES } from './language'
export type { Language } from './language'

export const LANGUAGE_STORAGE_KEY = 'deskhaven.language'

function browserLanguages(): readonly string[] {
  return navigator.languages.length > 0 ? navigator.languages : [navigator.language]
}

/** Keeps <html lang> in sync so screen readers and hyphenation use the UI language. */
function syncDocumentLanguage(language: string): void {
  document.documentElement.lang = language
}

export async function setLanguage(language: Language): Promise<void> {
  writeStorage(LANGUAGE_STORAGE_KEY, language)
  await i18n.changeLanguage(language)
}

export function currentLanguage(): Language {
  return isLanguage(i18n.resolvedLanguage) ? i18n.resolvedLanguage : FALLBACK_LANGUAGE
}

i18n.on('languageChanged', syncDocumentLanguage)

// Resources are bundled, so initialisation is synchronous: the first render
// already has translations and no Suspense boundary is needed.
void i18n.use(initReactI18next).init({
  resources,
  lng: detectLanguage(readStorage(LANGUAGE_STORAGE_KEY), browserLanguages()),
  fallbackLng: FALLBACK_LANGUAGE,
  supportedLngs: SUPPORTED_LANGUAGES,
  initAsync: false,
  interpolation: { escapeValue: false },
})
syncDocumentLanguage(currentLanguage())

export default i18n
