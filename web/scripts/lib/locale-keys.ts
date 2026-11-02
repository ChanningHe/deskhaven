// Pure helpers behind `pnpm i18n:check`. Kept free of I/O so they can be unit
// tested and run directly by Node's TypeScript type stripping.

export interface LocaleProblem {
  locale: string
  missing: string[]
}

export interface PlaceholderMismatch {
  key: string
  placeholders: Record<string, string[]>
}

// i18next interpolation: {{name}}, {{ name }}, {{name, format}}.
const PLACEHOLDER_PATTERN = /\{\{\s*([^\s,}]+)[^}]*\}\}/g

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** [dotted key, string value] for every leaf; throws on any non-string leaf. */
export function flattenEntries(tree: unknown, prefix = ''): [string, string][] {
  if (!isPlainObject(tree)) {
    throw new TypeError(`expected an object at "${prefix || '<root>'}"`)
  }
  return Object.entries(tree).flatMap(([key, value]): [string, string][] => {
    const path = prefix ? `${prefix}.${key}` : key
    if (typeof value === 'string') return [[path, value]]
    if (isPlainObject(value)) return flattenEntries(value, path)
    throw new TypeError(`expected a string or object at "${path}"`)
  })
}

export function flattenKeys(tree: unknown): string[] {
  return flattenEntries(tree).map(([key]) => key)
}

/** Sorted, de-duplicated interpolation variable names used in a message. */
export function placeholdersOf(message: string): string[] {
  const names = [...message.matchAll(PLACEHOLDER_PATTERN)].map((match) => match[1] ?? '')
  return [...new Set(names)].sort()
}

/** For each locale, the keys that exist in some other locale but not in it. */
export function findMissingKeys(locales: Record<string, unknown>): LocaleProblem[] {
  const keySets = Object.entries(locales).map(
    ([locale, tree]) => [locale, new Set(flattenKeys(tree))] as const,
  )
  const allKeys = new Set(keySets.flatMap(([, keys]) => [...keys]))

  return keySets
    .map(([locale, keys]) => ({
      locale,
      missing: [...allKeys].filter((key) => !keys.has(key)).sort(),
    }))
    .filter((problem) => problem.missing.length > 0)
}

/** Keys present in every locale whose placeholder sets differ between locales. */
export function findPlaceholderMismatches(locales: Record<string, unknown>): PlaceholderMismatch[] {
  const catalogs = Object.entries(locales).map(
    ([locale, tree]) => [locale, new Map(flattenEntries(tree))] as const,
  )
  const [first] = catalogs
  if (!first) return []

  return [...first[1].keys()].sort().flatMap((key) => {
    const placeholders: Record<string, string[]> = {}
    for (const [locale, messages] of catalogs) {
      const message = messages.get(key)
      if (message === undefined) return []
      placeholders[locale] = placeholdersOf(message)
    }
    const signatures = new Set(Object.values(placeholders).map((names) => names.join(',')))
    return signatures.size > 1 ? [{ key, placeholders }] : []
  })
}
