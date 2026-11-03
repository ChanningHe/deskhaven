import type { TFunction } from 'i18next'

import type { SettingView } from '@/api/generated/model'

const CATEGORY_LABELS = {
  auth: 'settings.categories.auth',
  smtp: 'settings.categories.smtp',
  devices: 'settings.categories.devices',
  addressbook: 'settings.categories.addressbook',
  audit: 'settings.categories.audit',
  signal: 'settings.categories.signal',
} as const

function isKnown(category: string): category is keyof typeof CATEGORY_LABELS {
  return category in CATEGORY_LABELS
}

/** The translated name of a category; one the console does not know shows as its key. */
export function categoryLabel(category: string, t: TFunction): string {
  return isKnown(category) ? t(CATEGORY_LABELS[category]) : category
}

export interface Category {
  name: string
  settings: SettingView[]
}

/** The settings grouped by category, in the order the server lists them. */
export function groupByCategory(settings: readonly SettingView[]): Category[] {
  const groups: Category[] = []
  for (const setting of settings) {
    const group = groups.find((g) => g.name === setting.category)
    if (group === undefined) groups.push({ name: setting.category, settings: [setting] })
    else group.settings.push(setting)
  }
  return groups
}
