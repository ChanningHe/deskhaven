import { KeyRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { useOidcProviders, useStartOidc } from '../api'
import { AuthError } from './auth-error'

/** One button per enabled OIDC provider; nothing when there is none. */
export function OidcButtons() {
  const { t } = useTranslation()
  const providers = useOidcProviders()
  const start = useStartOidc()
  const list = providers.data ?? []
  if (list.length === 0) return null

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <span className="h-px flex-1 bg-border" />
        {t('login.orContinueWith')}
        <span className="h-px flex-1 bg-border" />
      </div>
      {list.map(({ name }) => (
        <Button
          key={name}
          variant="outline"
          className="w-full"
          disabled={start.isPending}
          onClick={() => {
            start.mutate({ provider: name })
          }}
        >
          <KeyRound aria-hidden />
          {t('login.oidcButton', { name })}
        </Button>
      ))}
      <AuthError error={start.error} />
    </div>
  )
}
