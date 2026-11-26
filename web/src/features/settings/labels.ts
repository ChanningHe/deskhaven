import type { TFunction } from 'i18next'

import type { SettingView } from '@/api/generated/model'
import en from '@/i18n/locales/en.json'
import type { TranslationResources } from '@/i18n/resources'

type ItemTree = TranslationResources['settings']['items']

type ItemKey<T, Prefix extends string = ''> = {
  [K in keyof T & string]: T[K] extends { name: string; description: string }
    ? `${Prefix}${K}`
    : ItemKey<T[K], `${Prefix}${K}.`>
}[keyof T & string]

type KnownKey = ItemKey<ItemTree>

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

// `pnpm i18n:check` keeps every locale's `settings.items` identical in shape to `en`.
function isKnown(key: string): key is KnownKey {
  let node: unknown = en.settings.items
  for (const part of key.split('.')) node = isRecord(node) ? node[part] : undefined
  return isRecord(node) && typeof node.name === 'string'
}

/** The translated name of a setting; one the console does not know shows as its key. */
export function settingName(key: string, t: TFunction): string {
  return isKnown(key) ? t(`settings.items.${key}.name`) : key
}

/** The translated help of a setting; one the console does not know shows the server's text. */
export function settingHelp(
  setting: Pick<SettingView, 'key' | 'description'>,
  t: TFunction,
): string {
  return isKnown(setting.key) ? t(`settings.items.${setting.key}.description`) : setting.description
}
