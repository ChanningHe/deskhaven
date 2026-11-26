// Fails when a translation key exists in one locale but not in another, when a
// key's {{placeholders}} differ between locales, or when `settings.items` does not
// name and describe exactly the settings of the server's registry snapshot.
// Usage: pnpm i18n:check
import { readdirSync, readFileSync } from 'node:fs'
import { basename } from 'node:path'

import { findMissingKeys, findPlaceholderMismatches, flattenKeys } from './lib/locale-keys.ts'

const LOCALES_DIR = new URL('../src/i18n/locales/', import.meta.url)
const SETTINGS_SNAPSHOT = new URL(
  '../../crates/deskhaven-config/tests/snapshots/settings_schema.json',
  import.meta.url,
)
const SETTING_ITEMS = 'settings.items.'
const SETTING_FIELDS = ['name', 'description']
const MIN_LOCALES = 2

function loadLocales(): Record<string, unknown> {
  const files = readdirSync(LOCALES_DIR).filter((file) => file.endsWith('.json'))
  return Object.fromEntries(
    files.map((file): [string, unknown] => [
      basename(file, '.json'),
      JSON.parse(readFileSync(new URL(file, LOCALES_DIR), 'utf8')),
    ]),
  )
}

function settingKeys(): string[] {
  const snapshot = JSON.parse(readFileSync(SETTINGS_SNAPSHOT, 'utf8')) as {
    settings: { key: string }[]
  }
  return snapshot.settings.map((setting) => setting.key)
}

function settingItemProblems(locales: Record<string, unknown>): number {
  const expected = new Set(
    settingKeys().flatMap((key) =>
      SETTING_FIELDS.map((field) => `${SETTING_ITEMS}${key}.${field}`),
    ),
  )
  let problems = 0
  for (const [locale, tree] of Object.entries(locales)) {
    const present = new Set(flattenKeys(tree).filter((key) => key.startsWith(SETTING_ITEMS)))
    const missing = [...expected].filter((key) => !present.has(key))
    const unknown = [...present].filter((key) => !expected.has(key)).sort()
    for (const [what, keys] of [
      ['is missing', missing],
      ['has keys for no registry setting', unknown],
    ] as const) {
      if (keys.length === 0) continue
      problems += keys.length
      process.stderr.write(`i18n:check: ${locale} ${what} (${keys.length}):\n`)
      for (const key of keys) process.stderr.write(`  - ${key}\n`)
    }
  }
  return problems
}

function main(): number {
  const locales = loadLocales()
  const names = Object.keys(locales)
  if (names.length < MIN_LOCALES) {
    process.stderr.write(
      `i18n:check: expected at least ${MIN_LOCALES} locales, found ${names.length}\n`,
    )
    return 1
  }

  const problems = findMissingKeys(locales)
  for (const { locale, missing } of problems) {
    process.stderr.write(`i18n:check: ${locale} is missing ${missing.length} key(s):\n`)
    for (const key of missing) process.stderr.write(`  - ${key}\n`)
  }

  const mismatches = findPlaceholderMismatches(locales)
  for (const { key, placeholders } of mismatches) {
    process.stderr.write(`i18n:check: placeholders differ for "${key}":\n`)
    for (const [locale, names] of Object.entries(placeholders)) {
      process.stderr.write(`  - ${locale}: ${names.length > 0 ? names.join(', ') : '(none)'}\n`)
    }
  }
  const itemProblems = settingItemProblems(locales)
  if (problems.length > 0 || mismatches.length > 0 || itemProblems > 0) return 1

  const keyCount = flattenKeys(locales[names[0] ?? '']).length
  process.stdout.write(`i18n:check: ${keyCount} keys consistent across ${names.join(', ')}\n`)
  return 0
}

try {
  process.exitCode = main()
} catch (error) {
  process.stderr.write(`i18n:check: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
