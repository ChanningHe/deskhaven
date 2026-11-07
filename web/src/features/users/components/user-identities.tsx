import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { unlinkUserIdentity } from '@/api/generated/users/users'
import type { IdentityView, UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'

import { invalidateAccounts } from '../invalidate'

interface UserIdentitiesProps {
  user: UserView
  identities: IdentityView[]
  /** The viewer may manage the account (only an owner manages an owner). */
  editable: boolean
}

/**
 * The account's linked OIDC identities. Administrators view and unlink; linking is the
 * account holder's own step on their account page.
 */
export function UserIdentities({ user, identities, editable }: UserIdentitiesProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [target, setTarget] = useState<IdentityView | undefined>(undefined)
  const [error, setError] = useState<unknown>(undefined)

  const unlink = async () => {
    if (target === undefined) return
    setError(undefined)
    try {
      await unlinkUserIdentity(user.id, target.id)
      await invalidateAccounts(queryClient)
    } catch (caught) {
      setError(caught)
    }
  }

  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium">{t('users.drawer.identities')}</h3>
      {identities.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('users.drawer.noIdentities')}</p>
      ) : (
        <ul className="grid gap-1 text-xs">
          {identities.map((identity) => (
            <li key={identity.id} className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{identity.provider_display_name}</span>
              <span className="text-muted-foreground">{identity.email ?? identity.subject}</span>
              <Badge variant="outline">{t(`me.identities.via.${identity.linked_via}`)}</Badge>
              {editable ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="ms-auto"
                  onClick={() => {
                    setTarget(identity)
                  }}
                >
                  {t('users.drawer.unlink')}
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {error === undefined ? null : <ErrorAlert error={error} />}
      <ConfirmDialog
        open={target !== undefined}
        onOpenChange={(open) => {
          if (!open) setTarget(undefined)
        }}
        title={t('users.confirm.unlink.title')}
        description={t('users.confirm.unlink.description', {
          provider: target?.provider_display_name ?? '',
          username: user.username,
        })}
        confirmLabel={t('users.confirm.unlink.confirm')}
        destructive
        onConfirm={unlink}
      />
    </section>
  )
}
