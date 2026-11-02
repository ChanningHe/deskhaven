import * as React from 'react'
import { cn } from 'cn'

// `htmlFor` is passed explicitly (not via the spread) so jsx-a11y can verify
// that every label is associated with a control.
function Label({ className, htmlFor, ...props }: React.ComponentProps<'label'>) {
  return (
    <label
      data-slot="label"
      htmlFor={htmlFor}
      className={cn(
        'flex items-center gap-2 text-sm leading-none font-medium select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50',
        className,
      )}
      {...props}
    />
  )
}

export { Label }
