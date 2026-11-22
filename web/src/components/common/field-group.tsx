import type { ReactNode } from 'react'

interface FieldGroupProps {
  title: string
  description?: string
  children: ReactNode
}

/** A set of related fields inside a card: name and description on the left, fields on the right. */
export function FieldGroup({ title, description, children }: FieldGroupProps) {
  return (
    <div className="grid gap-4 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)] sm:gap-8">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">{title}</h3>
        {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      </div>
      <div className="w-full max-w-xl min-w-0 space-y-4">{children}</div>
    </div>
  )
}
