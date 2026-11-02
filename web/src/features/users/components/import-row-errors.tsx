import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'

import type { ImportRowError } from '@/api/generated/model'

const KNOWN_CODES = ['columns', 'invalid', 'taken', 'owner_required'] as const
type KnownCode = (typeof KNOWN_CODES)[number]

function isKnown(code: string): code is KnownCode {
  return (KNOWN_CODES as readonly string[]).includes(code)
}

function describe(row: ImportRowError, t: TFunction): string {
  const problem = isKnown(row.code) ? t(`users.import.codes.${row.code}`) : row.code
  if (row.field === null || row.field === undefined) return problem
  return t('users.import.fieldProblem', { field: row.field, problem })
}

/** Why an import was rejected, one line per CSV row problem (nothing was created). */
export function ImportRowErrors({ rows }: { rows: ImportRowError[] }) {
  const { t } = useTranslation()
  return (
    <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-3">
      <p className="text-sm font-medium text-destructive">{t('users.import.rejected')}</p>
      <ul className="mt-2 max-h-40 space-y-1 overflow-y-auto text-xs">
        {rows.map((row) => (
          <li key={`${row.line}-${row.field ?? ''}-${row.code}`}>
            {t('users.import.line', { line: row.line })}: {describe(row, t)}
          </li>
        ))}
      </ul>
    </div>
  )
}
