import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { unlinkUserIdentity } from '@/api/generated/users/users'
import type { IdentityView, UserView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
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
  // `target` outlives `open`: the dialog keeps its texts while it animates closed.
  const [target, setTarget] = useState<IdentityView | undefined>(undefined)
  const [open, setOpen] = useState(false)

  // A rejection propagates: the confirmation shows it and stays open.
  const unlink = async () => {
    if (target === undefined) return
    await unlinkUserIdentity(user.id, target.id)
    await invalidateAccounts(queryClient)
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
              <Badge variant="outline">{t(`users.drawer.via.${identity.linked_via}`)}</Badge>
              {editable ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="ms-auto"
                  aria-label={t('me.identities.unlinkNamed', {
                    provider: identity.provider_display_name,
                  })}
                  onClick={() => {
                    setTarget(identity)
                    setOpen(true)
                  }}
                >
                  {t('users.drawer.unlink')}
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title={t('users.confirm.unlink.title')}
        description={
          <>
            <span className="block">
              {t('users.confirm.unlink.description', {
                provider: target?.provider_display_name ?? '',
                username: user.username,
              })}
            </span>
            {user.email_verified ? (
              <span className="mt-2 block">{t('users.confirm.unlink.emailNote')}</span>
            ) : null}
          </>
        }
        confirmLabel={t('users.confirm.unlink.confirm')}
        destructive
        onConfirm={unlink}
      />
    </section>
  )
}
