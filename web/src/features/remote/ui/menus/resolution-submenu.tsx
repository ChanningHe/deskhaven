import { useTranslation } from 'react-i18next'

import {
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import { sameSize, type Size } from '../../session/resolution'
import { useStore } from '../use-remote'

const ORIGINAL = 'original'

const label = (size: Size) => `${size.width}×${size.height}`

export function ResolutionSubmenu({ session, viewOnly }: { session: Session; viewOnly: boolean }) {
  const { t } = useTranslation()
  const peer = useStore(session.peer)
  const { keyboard } = useStore(session.permissions)
  if (!peer || !keyboard || viewOnly || peer.resolutions.length < 2) return null
  const { display, originalResolution: original } = peer
  const current = display && original && sameSize(display, original) ? ORIGINAL : null
  const sizes = peer.resolutions.filter((r) => !original || !sameSize(r, original))
  const value = current ?? (display ? label(display) : '')
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>
          <span className="flex flex-1 justify-between gap-3">
            {t('remote.toolbar.resolution')}
            {display ? (
              <span className="text-muted-foreground tabular-nums">{label(display)}</span>
            ) : null}
          </span>
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="max-h-72 overflow-y-auto">
          <DropdownMenuRadioGroup
            value={value}
            onValueChange={(picked: unknown) => {
              const size = picked === ORIGINAL ? original : sizes.find((s) => label(s) === picked)
              if (size) session.changeResolution(size.width, size.height)
            }}
          >
            {original ? (
              <DropdownMenuRadioItem value={ORIGINAL} closeOnClick>
                {t('remote.toolbar.resolutionOriginal', { size: label(original) })}
              </DropdownMenuRadioItem>
            ) : null}
            {sizes.map((size) => (
              <DropdownMenuRadioItem key={label(size)} value={label(size)} closeOnClick>
                {label(size)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    </>
  )
}
