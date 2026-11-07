import { useQueryClient } from '@tanstack/react-query'
import { KeyRound } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useStartOidcLink, useUnlinkMyIdentity } from '@/api/generated/me/me'
import type { IdentityView, MyAccount } from '@/api/generated/model'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useOidcProviders } from '@/features/auth/api'
import { assignLocation } from '@/lib/browser-location'

import { refreshAccount } from '../invalidate'
import { ReauthDialog } from './reauth-dialog'
import { Section } from './section'

type Pending = { kind: 'link'; provider: string } | { kind: 'unlink'; identity: IdentityView }

/** `provider` (`subject` or address) as the list shows it. */
function identityLabel(identity: IdentityView): string {
  return identity.email ?? identity.subject
}

function IdentityRow({
  identity,
  canUnlink,
  onUnlink,
}: {
  identity: IdentityView
  canUnlink: boolean
  onUnlink: () => void
}) {
  const { t } = useTranslation()
  return (
    <li className="flex flex-wrap items-center gap-2 text-sm">
      <span className="font-medium">{identity.provider_display_name}</span>
      <span className="text-muted-foreground">{identityLabel(identity)}</span>
      <Badge variant="outline">{t(`me.identities.via.${identity.linked_via}`)}</Badge>
      {canUnlink ? (
        <Button size="sm" variant="outline" className="ms-auto" onClick={onUnlink}>
          {t('me.identities.unlink')}
        </Button>
      ) : null}
    </li>
  )
}

/**
 * "Linked sign-ins": the OIDC identities that sign in to this account, linking one more
 * (password, and a code while TOTP is on, then the identity provider) and unlinking
 * (password). Signing in through a linked identity asks for no local second factor.
 */
export function IdentitiesCard({ account }: { account: MyAccount }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const providers = useOidcProviders()
  const start = useStartOidcLink()
  const unlink = useUnlinkMyIdentity()
  const [pending, setPending] = useState<Pending | undefined>(undefined)
  const linked = new Set(account.identities.map((i) => i.provider))
  const linkable = (providers.data ?? []).filter(({ name }) => !linked.has(name))
  const close = (open: boolean) => {
    if (!open) setPending(undefined)
  }

  return (
    <Section title={t('me.identities.title')} description={t('me.identities.description')}>
      <p role="note" className="text-sm text-amber-700 dark:text-amber-400">
        {t('me.identities.warning')}
      </p>
      {account.identities.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('me.identities.none')}</p>
      ) : (
        <ul className="space-y-2">
          {account.identities.map((identity) => (
            <IdentityRow
              key={identity.id}
              identity={identity}
              canUnlink={account.can_link_oidc}
              onUnlink={() => {
                setPending({ kind: 'unlink', identity })
              }}
            />
          ))}
        </ul>
      )}
      {account.can_link_oidc ? (
        <div className="flex flex-wrap gap-2">
          {linkable.map(({ name }) => (
            <Button
              key={name}
              size="sm"
              variant="outline"
              onClick={() => {
                setPending({ kind: 'link', provider: name })
              }}
            >
              <KeyRound aria-hidden />
              {t('me.identities.link', { name })}
            </Button>
          ))}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t('me.identities.notLinkable')}</p>
      )}
      <ReauthDialog
        open={pending?.kind === 'link'}
        onOpenChange={close}
        title={t('me.identities.linkTitle')}
        description={t('me.identities.linkDescription')}
        submitLabel={t('me.identities.linkSubmit')}
        askCode={account.totp_enabled}
        onSubmit={async (currentPassword, code) => {
          if (pending?.kind !== 'link') return
          const data = {
            provider: pending.provider,
            current_password: currentPassword,
            code: account.totp_enabled ? code : null,
          }
          const { url } = await start.mutateAsync({ data })
          assignLocation(url)
        }}
      />
      <ReauthDialog
        open={pending?.kind === 'unlink'}
        onOpenChange={close}
        title={t('me.identities.unlinkTitle')}
        description={[
          t('me.identities.unlinkDescription'),
          pending?.kind === 'unlink' && pending.identity.linked_via === 'email'
            ? t('me.identities.unlinkEmailNote')
            : '',
        ].join(' ')}
        submitLabel={t('me.identities.unlinkSubmit')}
        askCode={false}
        onSubmit={async (currentPassword) => {
          if (pending?.kind !== 'unlink') return
          const data = { current_password: currentPassword }
          await unlink.mutateAsync({ id: pending.identity.id, data })
          await refreshAccount(queryClient)
        }}
      />
    </Section>
  )
}
