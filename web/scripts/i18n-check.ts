// Fails when a translation key exists in one locale but not in another, or
// when a key's {{placeholders}} differ between locales.
// Usage: pnpm i18n:check
import { readdirSync, readFileSync } from 'node:fs'
import { basename } from 'node:path'

import { findMissingKeys, findPlaceholderMismatches, flattenKeys } from './lib/locale-keys.ts'

const LOCALES_DIR = new URL('../src/i18n/locales/', import.meta.url)
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
  if (problems.length > 0 || mismatches.length > 0) return 1

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
