import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'
import { fieldProblemsOf } from '@/features/settings/conflicts'

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

/** A prominent caution (security trade-offs the admin must read). */
export function Warning({ children }: { children: ReactNode }) {
  return (
    <p className="flex items-start gap-2 rounded-lg bg-warning/10 px-3 py-2 text-sm text-warning">
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

/** A titled part of the identity page. */
export function Section({
  id,
  title,
  description,
  actions,
  children,
}: {
  id: string
  title: string
  description: string
  actions?: ReactNode
  children: ReactNode
}) {
  return (
    <section aria-labelledby={id} className="space-y-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="space-y-1">
          <h2 id={id} className="text-base font-semibold">
            {title}
          </h2>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
        {actions === undefined ? null : <div className="shrink-0">{actions}</div>}
      </div>
      {children}
    </section>
  )
}
