// Pure helpers behind `pnpm i18n:check`. Kept free of I/O so they can be unit
// tested and run directly by Node's TypeScript type stripping.

export interface LocaleProblem {
  locale: string
  missing: string[]
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Dotted key paths of every string leaf; throws on any non-string leaf. */
export function flattenKeys(tree: unknown, prefix = ''): string[] {
  if (!isPlainObject(tree)) {
    throw new TypeError(`expected an object at "${prefix || '<root>'}"`)
  }
  return Object.entries(tree).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key
    if (typeof value === 'string') return [path]
    if (isPlainObject(value)) return flattenKeys(value, path)
    throw new TypeError(`expected a string or object at "${path}"`)
  })
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
