// The server stays the judge; the browser only flags what it can already tell is wrong.

import type { SchemaBound, SettingView } from '@/api/generated/model'

export type Editor = string | boolean

const LIST_SEPARATOR = '\n'
const LIST_SPLIT = /[\n,]/
/** `15m`, `1h30m`, `7d`, `2 weeks` or a plain `0`; a bare number has no unit. */
const DURATION_PATTERN = /^(0|(\d+\s*[a-zµ]+\s*)+)$/i
const WHOLE_NUMBER = /^\d+$/

export type Problem =
  | { kind: 'required' }
  | { kind: 'whole_number' }
  | { kind: 'range'; min: number; max: number; zero: boolean }
  | { kind: 'duration' }
  | { kind: 'max_length'; max: number }
  | { kind: 'max_items'; max: number }
  | { kind: 'variant' }

export function listItems(text: string): string[] {
  return text
    .split(LIST_SPLIT)
    .map((item) => item.trim())
    .filter((item) => item !== '')
}

export function toEditor(setting: SettingView): Editor {
  const { value } = setting
  switch (setting.schema.type) {
    case 'bool':
      return value === true
    case 'secret':
      // Write-only: the field starts empty, whether or not a secret is set.
      return ''
    case 'host_port_list':
      return Array.isArray(value) ? value.filter(isString).join(LIST_SEPARATOR) : ''
    default:
      return typeof value === 'string' || typeof value === 'number' ? String(value) : ''
  }
}

function isString(value: unknown): value is string {
  return typeof value === 'string'
}

export function fromEditor(setting: SettingView, editor: Editor): unknown {
  switch (setting.schema.type) {
    case 'bool':
      return editor === true
    case 'u32':
      return Number(editor)
    case 'host_port_list':
      return listItems(String(editor))
    default:
      return String(editor)
  }
}

export function isSecretSet(setting: SettingView): boolean {
  const { value } = setting
  return typeof value === 'object' && value !== null && 'set' in value && value.set === true
}

function bound(value: SchemaBound | null | undefined, fallback: number): number {
  return typeof value === 'number' ? value : fallback
}

export function problemOf(setting: SettingView, editor: Editor): Problem | undefined {
  const { schema } = setting
  const text = String(editor)
  switch (schema.type) {
    case 'u32':
      return numberProblem(setting, text.trim())
    case 'duration':
      return DURATION_PATTERN.test(text.trim()) ? undefined : { kind: 'duration' }
    case 'enum':
      return schema.variants?.includes(text) === true ? undefined : { kind: 'variant' }
    case 'string':
    case 'secret':
      return schema.max_length != null && Array.from(text).length > schema.max_length
        ? { kind: 'max_length', max: schema.max_length }
        : undefined
    case 'host_port_list':
      return schema.max_items != null && listItems(text).length > schema.max_items
        ? { kind: 'max_items', max: schema.max_items }
        : undefined
    default:
      return undefined
  }
}

function numberProblem(setting: SettingView, text: string): Problem | undefined {
  if (text === '') return { kind: 'required' }
  if (!WHOLE_NUMBER.test(text)) return { kind: 'whole_number' }
  const { schema } = setting
  const min = bound(schema.min, 0)
  const max = bound(schema.max, Number.MAX_SAFE_INTEGER)
  const value = Number(text)
  const zero = schema.zero_allowed === true
  const inRange = (value >= min && value <= max) || (zero && value === 0)
  return inRange ? undefined : { kind: 'range', min, max, zero }
}

export function sameEditor(setting: SettingView, a: Editor, b: Editor): boolean {
  switch (setting.schema.type) {
    case 'host_port_list':
      return listItems(String(a)).join(LIST_SEPARATOR) === listItems(String(b)).join(LIST_SEPARATOR)
    case 'bool':
      return a === b
    default:
      return String(a).trim() === String(b).trim()
  }
}
