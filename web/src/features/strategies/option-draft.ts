// Mirrors the admin API's checks: option names `[A-Za-z0-9._-]`, typed catalog values, bounded sizes.

import type { CatalogOptionView, StrategyView } from '@/api/generated/model'

/**
 * An empty value makes the client delete the option (`reset`); `kept` is a secret the server holds
 * and withholds. An absent key is not managed by the strategy.
 */
export type OptionValue = { kind: 'set'; value: string } | { kind: 'reset' } | { kind: 'kept' }

export type OptionDraft = Record<string, OptionValue | undefined>

export const RESET: OptionValue = { kind: 'reset' }
export const KEPT: OptionValue = { kind: 'kept' }

export function setTo(value: string): OptionValue {
  return { kind: 'set', value }
}

export const KEY_MAX_LENGTH = 128
export const VALUE_MAX_LENGTH = 4096
/** The admin API's limit. */
export const MAX_OPTIONS = 256
const KEY_PATTERN = /^[A-Za-z0-9._-]+$/
/** Unicode `Cc` (C0, DEL and C1), as Rust's `char::is_control`: the admin API rejects them. */
const LAST_C0 = 0x1f
const FIRST_DEL_C1 = 0x7f
const LAST_C1 = 0x9f

export function optionCount(draft: OptionDraft): number {
  return Object.values(draft).filter((value) => value !== undefined).length
}

export function draftOf(strategy: StrategyView): OptionDraft {
  const draft: OptionDraft = {}
  for (const [key, value] of Object.entries(strategy.options)) {
    draft[key] = value === '' ? RESET : setTo(value)
  }
  for (const key of strategy.secret_options) draft[key] = KEPT
  return draft
}

/** The merge patch of the `changed` keys: a value, `''` (reset) or `null` (remove). */
export function patchOf(draft: OptionDraft, changed: string[]): Record<string, string | null> {
  const patch: Record<string, string | null> = {}
  for (const key of changed) {
    const option = draft[key]
    if (option === undefined) patch[key] = null
    else if (option.kind === 'set') patch[key] = option.value
    else if (option.kind === 'reset') patch[key] = ''
  }
  return patch
}

export type KeyProblem = 'empty' | 'tooLong' | 'charset'

export function keyProblem(key: string): KeyProblem | undefined {
  if (key === '') return 'empty'
  if (key.length > KEY_MAX_LENGTH) return 'tooLong'
  return KEY_PATTERN.test(key) ? undefined : 'charset'
}

export type ValueProblem =
  'tooLong' | 'control' | 'bool' | 'choice' | 'number' | 'range' | 'emptySet'

function hasControl(value: string): boolean {
  for (const c of value) {
    const code = c.charCodeAt(0)
    if (code <= LAST_C0 || (code >= FIRST_DEL_C1 && code <= LAST_C1)) return true
  }
  return false
}

/** Length in Unicode code points, as the server counts characters. */
function codePoints(value: string): number {
  return Array.from(value).length
}

function kindProblem(option: CatalogOptionView, value: string): ValueProblem | undefined {
  switch (option.kind) {
    case 'bool':
      return value === 'Y' || value === 'N' ? undefined : 'bool'
    case 'choice':
      return option.choices.some((c) => c.value === value) ? undefined : 'choice'
    case 'number': {
      if (!/^-?\d+$/.test(value)) return 'number'
      const n = Number(value)
      const low = option.min ?? Number.MIN_SAFE_INTEGER
      const high = option.max ?? Number.MAX_SAFE_INTEGER
      return n < low || n > high ? 'range' : undefined
    }
    case 'text':
      return undefined
  }
}

export function valueProblem(
  option: CatalogOptionView | undefined,
  value: string,
): ValueProblem | undefined {
  if (value === '') return undefined
  if (codePoints(value) > VALUE_MAX_LENGTH) return 'tooLong'
  if (hasControl(value)) return 'control'
  return option === undefined ? undefined : kindProblem(option, value)
}
