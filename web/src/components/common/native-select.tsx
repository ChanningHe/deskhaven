import type { ComponentProps } from 'react'

import { cn } from '@/lib/utils'

/**
 * A styled native `<select>`: accessible and keyboard-friendly out of the box,
 * for short option lists (filters, roles). Give it a label (`aria-label` or a
 * `<Label htmlFor>`).
 */
export function NativeSelect({ className, ...props }: ComponentProps<'select'>) {
  return (
    <select
      className={cn(
        'h-8 rounded-lg border border-input bg-transparent px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-input/30',
        className,
      )}
      {...props}
    />
  )
}
