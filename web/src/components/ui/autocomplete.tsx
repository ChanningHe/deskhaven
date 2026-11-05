import { Autocomplete as AutocompletePrimitive } from '@base-ui/react/autocomplete'
import { cn } from 'cn'

import { Input } from '@/components/ui/input'

/** The text field; it anchors the floating list. */
function AutocompleteInput({
  className,
  ...props
}: Omit<AutocompletePrimitive.Input.Props, 'className'> & { className?: string }) {
  return (
    <AutocompletePrimitive.Input
      data-slot="autocomplete-input"
      render={<Input className={className} />}
      {...props}
    />
  )
}

/** The floating list: portalled, so it overlays whatever surrounds the field, drawers included. */
function AutocompleteContent({
  align = 'start',
  alignOffset = 0,
  side = 'bottom',
  sideOffset = 4,
  className,
  ...props
}: AutocompletePrimitive.Popup.Props &
  Pick<AutocompletePrimitive.Positioner.Props, 'align' | 'alignOffset' | 'side' | 'sideOffset'>) {
  return (
    <AutocompletePrimitive.Portal>
      <AutocompletePrimitive.Positioner
        className="isolate z-50 outline-none"
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
      >
        <AutocompletePrimitive.Popup
          data-slot="autocomplete-content"
          className={cn(
            'z-50 max-h-(--available-height) w-(--anchor-width) min-w-(--anchor-width) origin-(--transform-origin) overflow-x-hidden overflow-y-auto rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 outline-none data-[side=bottom]:slide-in-from-top-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95',
            className,
          )}
          {...props}
        />
      </AutocompletePrimitive.Positioner>
    </AutocompletePrimitive.Portal>
  )
}

function AutocompleteList({ className, ...props }: AutocompletePrimitive.List.Props) {
  return (
    <AutocompletePrimitive.List
      data-slot="autocomplete-list"
      className={cn('grid gap-0.5 text-sm', className)}
      {...props}
    />
  )
}

/** Announced politely to screen readers; add `sr-only` to keep it from showing. */
function AutocompleteStatus({ className, ...props }: AutocompletePrimitive.Status.Props) {
  return (
    <AutocompletePrimitive.Status
      data-slot="autocomplete-status"
      className={cn('px-2 py-1 text-xs text-muted-foreground', className)}
      {...props}
    />
  )
}

function AutocompleteItem({ className, ...props }: AutocompletePrimitive.Item.Props) {
  return (
    <AutocompletePrimitive.Item
      data-slot="autocomplete-item"
      className={cn(
        'cursor-default rounded-md px-2 py-1 outline-hidden select-none data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:cursor-wait data-disabled:opacity-50',
        className,
      )}
      {...props}
    />
  )
}

export {
  AutocompleteInput,
  AutocompleteContent,
  AutocompleteList,
  AutocompleteStatus,
  AutocompleteItem,
}
