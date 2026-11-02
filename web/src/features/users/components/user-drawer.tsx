import { useTranslation } from 'react-i18next'

import { useGetUser } from '@/api/generated/users/users'
import type { UserDetail } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Badge } from '@/components/ui/badge'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { useMe } from '@/features/auth/session'
import { formatDateTime } from '@/lib/format'

import { canManage } from '../user-rules'
import { UserActions } from './user-actions'
import { UserEditForm } from './user-edit-form'

interface UserDrawerProps {
  /** The account to show; undefined closes the drawer. */
  userId: number | undefined
  onClose: () => void
}

/** Side drawer of one account: details, edit form, actions, groups and sessions. */
export function UserDrawer({ userId, onClose }: UserDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={userId !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-md">
        {userId === undefined ? null : <DrawerBody userId={userId} onDeleted={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({ userId, onDeleted }: { userId: number; onDeleted: () => void }) {
  const { t } = useTranslation()
  const query = useGetUser(userId)
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('users.drawer.title')}</SheetTitle>
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <SheetDescription>{t('common.loading')}</SheetDescription>
        )}
      </SheetHeader>
    )
  }
  return <Detail detail={query.data} onDeleted={onDeleted} />
}

function Detail({ detail, onDeleted }: { detail: UserDetail; onDeleted: () => void }) {
  const { t, i18n } = useTranslation()
  const me = useMe()
  const { user } = detail
  const editable = canManage(me, user.role)
  return (
    <>
      <SheetHeader>
        <SheetTitle>{user.username}</SheetTitle>
        <SheetDescription>{user.email ?? user.display_name}</SheetDescription>
        <div className="flex flex-wrap gap-1 pt-2">
          <Badge variant="outline">{t(`roles.${user.role}`)}</Badge>
          <Badge variant="outline">{t(`users.statuses.${user.status}`)}</Badge>
          <Badge variant="outline">{t(`users.sources.${user.auth_source}`)}</Badge>
          <Badge variant="outline">
            {user.totp_enabled ? t('users.tfaOn') : t('users.tfaOff')}
          </Badge>
        </div>
      </SheetHeader>
      <div className="grid gap-6 px-4 pb-6">
        {editable ? null : (
          <p className="rounded-lg bg-muted px-3 py-2 text-xs text-muted-foreground">
            {t('users.drawer.ownerReadOnly')}
          </p>
        )}
        <UserEditForm key={user.id} user={user} editable={editable} />
        {editable ? <UserActions user={user} onDeleted={onDeleted} /> : null}
        <section className="grid gap-2">
          <h3 className="text-sm font-medium">{t('users.drawer.groups')}</h3>
          {detail.groups.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t('users.drawer.noGroups')}</p>
          ) : (
            <ul className="flex flex-wrap gap-1">
              {detail.groups.map((group) => (
                <li key={group.id}>
                  <Badge variant="secondary">{group.name}</Badge>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section className="grid gap-2">
          <h3 className="text-sm font-medium">{t('users.drawer.sessions')}</h3>
          {detail.sessions.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t('users.drawer.noSessions')}</p>
          ) : (
            <ul className="grid gap-1 text-xs">
              {detail.sessions.map((session) => (
                <li key={session.id} className="flex justify-between gap-2">
                  <span>
                    {t(`users.drawer.sessionKinds.${session.kind}`)}
                    {session.rustdesk_id ? ` · ${session.rustdesk_id}` : ''}
                    {session.ip ? ` · ${session.ip}` : ''}
                  </span>
                  <span className="text-muted-foreground">
                    {formatDateTime(session.last_used_at, i18n.language)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </>
  )
}
