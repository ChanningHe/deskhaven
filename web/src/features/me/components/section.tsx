import { useId } from 'react'
import type { ReactNode } from 'react'

interface SectionProps {
  title: string
  description?: string
  children: ReactNode
}

/** A titled block of the account page. */
export function Section({ title, description, children }: SectionProps) {
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="space-y-3 rounded-xl border p-4">
      <div className="space-y-1">
        <h2 id={headingId} className="text-base font-medium">
          {title}
        </h2>
        {description === undefined ? null : (
          <p className="text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      {children}
    </section>
  )
}
