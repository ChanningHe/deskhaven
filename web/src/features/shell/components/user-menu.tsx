import { Link } from '@tanstack/react-router'
import { LogOut, UserRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useLogout } from '@/features/auth/api'
import { useMe } from '@/features/auth/session'

// Sign-out revokes the session server-side, then drops the cache (M3-02).
export function UserMenu() {
  const { t } = useTranslation()
  const me = useMe()
  const logout = useLogout()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="rounded-full"
            aria-label={t('user.menu')}
          />
        }
      >
        <span className="flex size-7 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <UserRound aria-hidden className="size-4" />
        </span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-52">
        <DropdownMenuGroup>
          <DropdownMenuLabel className="flex flex-col gap-0.5">
            <span className="truncate text-sm font-medium text-foreground">{me.display_name}</span>
            <span className="truncate">
              {me.username} · {t(`roles.${me.role}`)}
            </span>
          </DropdownMenuLabel>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem render={<Link to="/me" />}>
          <UserRound aria-hidden />
          {t('user.profile')}
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => {
            logout.mutate()
          }}
        >
          <LogOut aria-hidden />
          {t('user.signOut')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
