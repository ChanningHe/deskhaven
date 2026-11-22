import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'
import { fieldProblemsOf } from '@/features/settings/conflicts'
import { cn } from '@/lib/utils'

interface CheckFieldProps {
  label: string
  checked: boolean
  disabled?: boolean | undefined
  onChange: (checked: boolean) => void
}

/** A checkbox with its label. */
export function CheckField({ label, checked, disabled, onChange }: CheckFieldProps) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <Checkbox checked={checked} disabled={disabled} onCheckedChange={onChange} />
      {label}
    </label>
  )
}

const WARNING_TONE = {
  warning: 'bg-warning/10 text-warning',
  destructive: 'bg-destructive/10 text-destructive',
} as const

/** A prominent caution (security trade-offs the admin must read); `destructive` in red. */
export function Warning({
  children,
  tone = 'warning',
}: {
  children: ReactNode
  tone?: keyof typeof WARNING_TONE
}) {
  return (
    <p className={cn('flex items-start gap-2 rounded-lg px-3 py-2 text-sm', WARNING_TONE[tone])}>
      <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
      <span>{children}</span>
    </p>
  )
}

/** The server's reasons per field of a refused form (`validation_failed` details). */
export function FieldProblems({ error }: { error: unknown }) {
  const { t } = useTranslation()
  const problems = Object.entries(fieldProblemsOf(error))
  if (problems.length === 0) return null
  return (
    <ul className="space-y-1 text-xs text-destructive">
      {problems.flatMap(([field, reasons]) =>
        reasons.map((reason) => (
          <li key={`${field}:${reason}`}>
            <span className="font-mono">{field}</span> {t('settings.row.serverRefused', { reason })}
          </li>
        )),
      )}
    </ul>
  )
}
