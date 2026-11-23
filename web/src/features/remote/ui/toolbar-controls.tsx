import type { ComponentProps, ReactElement, ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

/** A control with its hint below it; the hint repeats or extends the control's name. */
function Hinted({ hint, render }: { hint: string; render: ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger render={render} />
      <TooltipContent side="bottom">{hint}</TooltipContent>
    </Tooltip>
  )
}

/** An icon-only button: its label is its accessible name, and its tooltip unless a hint is given. */
export function IconButton({
  label,
  hint = label,
  ...props
}: { label: string; hint?: string } & ComponentProps<typeof Button>) {
  return (
    <Hinted
      hint={hint}
      render={<Button variant="ghost" size="icon" aria-label={label} {...props} />}
    />
  )
}

export function ToolbarMenu({
  label,
  hint,
  icon,
  className,
  children,
}: {
  label: string
  hint: string
  icon: ReactNode
  className?: string
  children: ReactNode
}) {
  return (
    <DropdownMenu>
      <Hinted
        hint={hint}
        render={
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={label} />}>
            {icon}
          </DropdownMenuTrigger>
        }
      />
      <DropdownMenuContent className={cn('w-56', className)}>{children}</DropdownMenuContent>
    </DropdownMenu>
  )
}
