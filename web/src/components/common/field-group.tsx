import type { ReactNode } from 'react'

interface FieldGroupProps {
  title: string
  description?: string
  children: ReactNode
}

/** Two columns only from `lg`: from `md` the sidebar leaves too little width for both. */
export function FieldGroup({ title, description, children }: FieldGroupProps) {
  return (
    <div className="grid gap-4 not-first:border-t not-first:pt-6 lg:grid-cols-[minmax(0,13rem)_minmax(0,1fr)] lg:gap-10">
      <div className="space-y-1">
        <h3 className="text-sm leading-6 font-semibold">{title}</h3>
        {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      </div>
      <div className="w-full max-w-xl min-w-0 space-y-4">{children}</div>
    </div>
  )
}
