import type { ReactNode } from 'react'

import { Brand } from '@/features/shell/components/brand'
import { cn } from '@/lib/utils'

interface AuthLayoutProps {
  title: string
  description?: ReactNode
  children: ReactNode
  /** Wider card (setup, TOTP enrolment). */
  wide?: boolean
}

/** The centred card of the pages outside the console shell (sign-in, setup). */
export function AuthLayout({ title, description, children, wide = false }: AuthLayoutProps) {
  return (
    <main className="flex min-h-svh items-center justify-center bg-muted/40 px-4 py-12">
      <div
        className={cn(
          'w-full space-y-6 rounded-xl border bg-card p-6 shadow-sm',
          wide ? 'max-w-lg' : 'max-w-sm',
        )}
      >
        <Brand />
        <div className="space-y-1">
          <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
          {description === undefined ? null : (
            <p className="text-sm text-muted-foreground">{description}</p>
          )}
        </div>
        {children}
      </div>
    </main>
  )
}
