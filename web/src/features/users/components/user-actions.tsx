import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteUser,
  disableUser,
  enableUser,
  resetUser2fa,
  revokeUserSessions,
  unlockUser,
  useResetUserPassword,
} from '@/api/generated/users/users'
import type { UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Button } from '@/components/ui/button'
import { useMe } from '@/features/auth/session'

import { invalidateAccounts } from '../invalidate'
import { canDelete } from '../user-rules'

/** Actions that ask for confirmation first (08 §1.4). */
type Confirmed = 'disable' | 'revoke' | 'reset2fa' | 'resetPassword' | 'delete'

interface UserActionsProps {
  user: UserView
  onDeleted: () => void
}

/** Account actions of the drawer; only shown to someone who may manage the account. */
export function UserActions({ user, onDeleted }: UserActionsProps) {
  const { t } = useTranslation()
  const me = useMe()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState<Confirmed | null>(null)
  const [error, setError] = useState<unknown>(undefined)
  const [unlocked, setUnlocked] = useState(false)
  const reset = useResetUserPassword()
  const refresh = () => invalidateAccounts(queryClient)

  const run = async (action: () => Promise<unknown>) => {
    setError(undefined)
    try {
      await action()
      await refresh()
    } catch (caught) {
      setError(caught)
    }
  }

  const confirmed: Record<Confirmed, () => Promise<unknown>> = {
    disable: async () => disableUser(user.id).then(refresh),
    revoke: async () => revokeUserSessions(user.id).then(refresh),
    reset2fa: async () => resetUser2fa(user.id).then(refresh),
    resetPassword: async () => reset.mutateAsync({ id: user.id }).then(refresh),
    delete: async () =>
      deleteUser(user.id).then(() => {
        onDeleted()
        return refresh()
      }),
  }

  const ask = (action: Confirmed) => () => {
    setConfirming(action)
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium">{t('users.actions.title')}</h3>
      <div className="flex flex-wrap gap-2">
        {user.auth_source === 'local' ? (
          <Button variant="outline" size="sm" onClick={ask('resetPassword')}>
            {t('users.actions.resetPassword')}
          </Button>
        ) : null}
        {user.totp_enabled ? (
          <Button variant="outline" size="sm" onClick={ask('reset2fa')}>
            {t('users.actions.reset2fa')}
          </Button>
        ) : null}
        <Button variant="outline" size="sm" onClick={ask('revoke')}>
          {t('users.actions.revoke')}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={() =>
            void run(async () => {
              await unlockUser(user.id)
              setUnlocked(true)
            })
          }
        >
          {t('users.actions.unlock')}
        </Button>
        {user.status === 'active' ? (
          <Button variant="outline" size="sm" onClick={ask('disable')}>
            {t('users.actions.disable')}
          </Button>
        ) : (
          <Button variant="outline" size="sm" onClick={() => void run(() => enableUser(user.id))}>
            {t('users.actions.enable')}
          </Button>
        )}
        {canDelete(me, user) ? (
          <Button variant="destructive" size="sm" onClick={ask('delete')}>
            {t('users.actions.delete')}
          </Button>
        ) : null}
      </div>
      <p role="status" className="text-xs text-muted-foreground">
        {unlocked ? t('users.actions.unlocked') : null}
      </p>
      {error === undefined ? null : <ErrorAlert error={error} />}
      <ConfirmDialog
        open={confirming !== null}
        onOpenChange={(open) => {
          if (!open) setConfirming(null)
        }}
        title={confirming === null ? '' : t(`users.confirm.${confirming}.title`)}
        description={
          confirming === null
            ? undefined
            : t(`users.confirm.${confirming}.description`, { username: user.username })
        }
        confirmLabel={confirming === null ? '' : t(`users.confirm.${confirming}.confirm`)}
        {...(confirming === 'delete' ? { confirmText: user.username } : {})}
        destructive={confirming !== 'resetPassword'}
        onConfirm={confirming === null ? () => undefined : confirmed[confirming]}
      />
      <SecretRevealDialog
        open={reset.data !== undefined}
        title={t('users.actions.temporaryTitle', { username: user.username })}
        description={t('users.actions.temporaryDescription')}
        secret={reset.data?.temporary_password ?? ''}
        onDone={() => {
          reset.reset()
        }}
      />
    </section>
  )
}
