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
  updateUser,
} from '@/api/generated/users/users'
import type { UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { useMe } from '@/features/auth/session'

import { invalidateAccounts } from '../invalidate'
import { canDelete } from '../user-rules'

/** Actions that ask for confirmation first (08 §1.4). */
type Confirmed = 'disable' | 'revoke' | 'reset2fa' | 'resetPassword' | 'markVerified' | 'delete'

/** What the page does for the drawer's actions. */
export interface UserActionHandlers {
  onDeleted: () => void
  /** Resets the password; the page shows the temporary one (see `useResetPasswordReveal`). */
  onResetPassword: (user: UserView) => Promise<unknown>
}

interface UserActionsProps extends UserActionHandlers {
  user: UserView
  /** Linked OIDC identities: resets leave them in place, so the confirmations say so. */
  identityCount: number
}

/** Resets that leave linked sign-ins working (they ask for no local password or factor). */
const IDENTITIES_UNTOUCHED: readonly Confirmed[] = ['resetPassword', 'reset2fa']

/** Account actions of the drawer; only shown to someone who may manage the account. */
export function UserActions({ user, identityCount, onDeleted, onResetPassword }: UserActionsProps) {
  const { t } = useTranslation()
  const me = useMe()
  const queryClient = useQueryClient()
  // `action` outlives `open`: the dialog keeps its texts while it animates closed.
  const [action, setAction] = useState<Confirmed>('disable')
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(undefined)
  const [unlocked, setUnlocked] = useState(false)
  const refresh = () => invalidateAccounts(queryClient)

  /** A direct (unconfirmed) action; its button stays disabled until it settles. */
  const run = async (work: () => Promise<unknown>) => {
    if (busy) return
    setBusy(true)
    setError(undefined)
    try {
      await work()
      await refresh()
    } catch (caught) {
      setError(caught)
    } finally {
      setBusy(false)
    }
  }

  const confirmed: Record<Confirmed, () => Promise<unknown>> = {
    disable: async () => disableUser(user.id).then(refresh),
    revoke: async () => revokeUserSessions(user.id).then(refresh),
    reset2fa: async () => resetUser2fa(user.id).then(refresh),
    resetPassword: async () => onResetPassword(user).then(refresh),
    // The address the administrator checked: a changed one is refused (409).
    markVerified: async () =>
      updateUser(user.id, { email_verified: true, expected_email: user.email ?? '' }).then(refresh),
    delete: async () =>
      deleteUser(user.id).then(() => {
        onDeleted()
        return refresh()
      }),
  }

  const ask = (next: Confirmed) => () => {
    setAction(next)
    setConfirming(true)
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
        {user.totp_enabled && user.id !== me.id ? (
          <Button variant="outline" size="sm" onClick={ask('reset2fa')}>
            {t('users.actions.reset2fa')}
          </Button>
        ) : null}
        <Button variant="outline" size="sm" onClick={ask('revoke')}>
          {t('users.actions.revoke')}
        </Button>
        {user.email !== null && user.email !== undefined && !user.email_verified ? (
          <Button variant="outline" size="sm" onClick={ask('markVerified')}>
            {t('users.actions.markEmailVerified')}
          </Button>
        ) : null}
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          aria-busy={busy}
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
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            aria-busy={busy}
            onClick={() => void run(() => enableUser(user.id))}
          >
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
        open={confirming}
        onOpenChange={setConfirming}
        title={t(`users.confirm.${action}.title`)}
        description={
          <>
            <span className="block">
              {t(`users.confirm.${action}.description`, {
                username: user.username,
                email: user.email ?? '',
              })}
            </span>
            {identityCount > 0 && IDENTITIES_UNTOUCHED.includes(action) ? (
              <span className="mt-2 block">
                {t('users.confirm.identitiesNote', { count: identityCount })}
              </span>
            ) : null}
          </>
        }
        confirmLabel={t(`users.confirm.${action}.confirm`)}
        {...(action === 'delete' ? { confirmText: user.username } : {})}
        destructive={action !== 'resetPassword' && action !== 'markVerified'}
        onConfirm={confirmed[action]}
      />
    </section>
  )
}
