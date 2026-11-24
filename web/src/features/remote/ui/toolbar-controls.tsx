import type { ComponentProps, ReactElement, ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

// Below lg the toolbar has a row of its own and is often touched: larger targets fit there.
const TOOL = 'max-lg:size-10'

/** A control with its hint below it; the hint repeats or extends the control's name. */
function Hinted({ hint, render }: { hint: ReactNode; render: ReactElement }) {
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
  className,
  ...props
}: { label: string; hint?: ReactNode } & ComponentProps<typeof Button>) {
  return (
    <Hinted
      hint={hint}
      render={
        <Button
          variant="ghost"
          size="icon"
          aria-label={label}
          className={cn(TOOL, className)}
          {...props}
        />
      }
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
          <DropdownMenuTrigger
            render={<Button variant="ghost" size="icon" aria-label={label} className={TOOL} />}
          >
            {icon}
          </DropdownMenuTrigger>
        }
      />
      <DropdownMenuContent className={cn('w-60', className)}>{children}</DropdownMenuContent>
    </DropdownMenu>
  )
}

/** A menu checkbox drawn as a switch, so that "off" shows as plainly as "on". */
export function MenuSwitch({
  className,
  children,
  ...props
}: ComponentProps<typeof DropdownMenuCheckboxItem>) {
  return (
    <DropdownMenuCheckboxItem
      className={cn(
        'group/switch pr-1.5 *:data-[slot=dropdown-menu-checkbox-item-indicator]:hidden',
        className,
      )}
      {...props}
    >
      <span className="min-w-0 flex-1">{children}</span>
      <span
        aria-hidden
        className="inline-flex h-4 w-7 shrink-0 items-center rounded-full bg-foreground/20 p-0.5 transition-colors group-data-checked/switch:bg-primary motion-reduce:transition-none"
      >
        <span className="size-3 rounded-full bg-background shadow-xs transition-transform group-data-checked/switch:translate-x-3 group-data-checked/switch:bg-primary-foreground motion-reduce:transition-none dark:bg-muted-foreground dark:group-data-checked/switch:bg-primary-foreground" />
      </span>
    </DropdownMenuCheckboxItem>
  )
}
