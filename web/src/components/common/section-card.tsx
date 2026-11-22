import { useId } from 'react'
import type { ReactNode, Ref } from 'react'

interface SectionCardProps {
  title: string
  description?: string
  /** Badges next to the title: saved facts (source, enabled state), never controls. */
  status?: ReactNode
  /** The card's primary actions, at the right end of the title bar. */
  actions?: ReactNode
  /** Bottom action bar (save button, result notice). */
  footer?: ReactNode
  /** Makes the heading a programmatic focus target (in-page navigation lands on it). */
  titleRef?: Ref<HTMLHeadingElement>
  children: ReactNode
}

/** One self-contained part of a page: a bordered card whose h2 names the region. */
export function SectionCard({
  title,
  description,
  status,
  actions,
  footer,
  titleRef,
  children,
}: SectionCardProps) {
  const headingId = useId()
  return (
    <section
      aria-labelledby={headingId}
      className="min-w-0 rounded-xl border bg-card text-card-foreground shadow-xs"
    >
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3 rounded-t-xl border-b bg-muted/50 px-4 py-4 sm:px-6">
        <div className="min-w-0 flex-1 basis-64 space-y-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <h2
              id={headingId}
              ref={titleRef}
              tabIndex={titleRef === undefined ? undefined : -1}
              className="text-base leading-6 font-semibold"
            >
              {title}
            </h2>
            {status ? (
              <div data-slot="section-card-status" className="flex flex-wrap items-center gap-1.5">
                {status}
              </div>
            ) : null}
          </div>
          {description ? (
            <p className="max-w-prose text-sm text-muted-foreground">{description}</p>
          ) : null}
        </div>
        {actions ? (
          <div data-slot="section-card-actions" className="flex shrink-0 items-center gap-2">
            {actions}
          </div>
        ) : null}
      </div>
      <div className="px-4 py-5 sm:px-6 sm:py-6">{children}</div>
      {footer ? (
        <div
          data-slot="section-card-footer"
          className="flex flex-wrap items-center gap-3 rounded-b-xl border-t bg-muted/50 px-4 py-3 sm:px-6"
        >
          {footer}
        </div>
      ) : null}
    </section>
  )
}
