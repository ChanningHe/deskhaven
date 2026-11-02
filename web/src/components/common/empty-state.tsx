import type { LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'

const HEADING_TAGS = { 1: 'h1', 2: 'h2', 3: 'h3' } as const

interface EmptyStateProps {
  icon: LucideIcon
  title: string
  description?: string
  /** The next step for the user (08 §4: every empty state offers one). */
  action?: ReactNode
  /**
   * Level of the title heading. 2 under a page header (default); 1 when the
   * empty state *is* the page (not found, route error).
   */
  headingLevel?: keyof typeof HEADING_TAGS
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  headingLevel = 2,
}: EmptyStateProps) {
  const Heading = HEADING_TAGS[headingLevel]
  return (
    <section className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed px-6 py-16 text-center">
      <div className="flex size-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
        <Icon aria-hidden className="size-5" />
      </div>
      <div className="max-w-sm space-y-1">
        <Heading className="text-sm font-medium">{title}</Heading>
        {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      </div>
      {action ? <div className="flex flex-wrap justify-center gap-2 pt-1">{action}</div> : null}
    </section>
  )
}
