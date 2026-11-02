import en from './locales/en.json'
import zhCN from './locales/zh-CN.json'

// `en` is the source of truth for the key shape: every typed `t()` call is
// checked against it, and assigning `zhCN` to it makes `tsc` fail when the
// Chinese catalog misses a key. Extra keys are caught by `pnpm i18n:check`.
export type TranslationResources = typeof en

const zhCNTranslation: TranslationResources = zhCN

export const resources = {
  'zh-CN': { translation: zhCNTranslation },
  en: { translation: en },
}
