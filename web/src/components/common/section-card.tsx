import { useId } from 'react'
import type { ReactNode } from 'react'

interface SectionCardProps {
  title: string
  description?: string
  /** Badges next to the title: saved facts (source, enabled state), never controls. */
  status?: ReactNode
  /** The card's primary actions, at the right end of the title bar. */
  actions?: ReactNode
  /** Bottom action bar (save button, result notice). */
  footer?: ReactNode
  children: ReactNode
}

/** One self-contained part of a page: a bordered card whose h2 names the region. */
export function SectionCard({
  title,
  description,
  status,
  actions,
  footer,
  children,
}: SectionCardProps) {
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="rounded-xl border bg-card text-card-foreground">
      <div className="flex flex-col gap-3 border-b px-4 py-3 sm:flex-row sm:items-start sm:justify-between sm:px-6 sm:py-4">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 id={headingId} className="text-base font-semibold">
              {title}
            </h2>
            {status ? (
              <div data-slot="section-card-status" className="flex items-center gap-1.5">
                {status}
              </div>
            ) : null}
          </div>
          {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
        </div>
        {actions ? (
          <div data-slot="section-card-actions" className="flex shrink-0 items-center gap-2">
            {actions}
          </div>
        ) : null}
      </div>
      <div className="px-4 py-4 sm:px-6 sm:py-5">{children}</div>
      {footer ? (
        <div
          data-slot="section-card-footer"
          className="flex flex-wrap items-center gap-3 border-t px-4 py-3 sm:px-6"
        >
          {footer}
        </div>
      ) : null}
    </section>
  )
}
