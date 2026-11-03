// Display forms of settings: the source badge's model and a value as short text.

import type { TFunction } from 'i18next'

import type { SettingView, SourceView } from '@/api/generated/model'
import type { ValueSource } from '@/components/common/source-badge'

/** The badge model of an API source: pins read "locked", stored values "modified". */
export function valueSource(source: SourceView): ValueSource {
  switch (source.kind) {
    case 'default':
      return { kind: 'default' }
    case 'file':
      return { kind: 'file', path: source.path }
    case 'env':
      return { kind: 'env', name: source.var }
    case 'cli':
      return { kind: 'file' }
    case 'database': {
      const by = source.by_username ?? (source.by == null ? undefined : `#${source.by}`)
      return {
        kind: 'modified',
        ...(by === undefined ? {} : { by }),
        at: new Date(source.at).toISOString(),
      }
    }
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * A value as short text. Secrets are never shown, only whether they are set; empty text and
 * empty lists read as "none".
 */
export function displayValue(setting: SettingView, value: unknown, t: TFunction): string {
  if (setting.schema.type === 'secret' || isRecord(value)) {
    const set = isRecord(value) && value.set === true
    return set ? t('settings.value.secretSet') : t('settings.value.secretUnset')
  }
  if (typeof value === 'boolean') return value ? t('settings.value.on') : t('settings.value.off')
  if (Array.isArray(value)) {
    return value.length === 0 ? t('settings.value.none') : value.map(String).join(', ')
  }
  if (value === '' || value === null || value === undefined) return t('settings.value.none')
  return typeof value === 'string' || typeof value === 'number'
    ? String(value)
    : t('settings.value.none')
}
