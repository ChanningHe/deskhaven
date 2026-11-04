import { useTranslation } from 'react-i18next'

import type { ApiTokenView, Me } from '@/api/generated/model'
import { Button } from '@/components/ui/button'

import { canRevoke, isOwnerLevel } from '../token-rules'

interface RevokeButtonProps {
  me: Pick<Me, 'role'>
  token: ApiTokenView
  onRevoke: (token: ApiTokenView) => void
}

/**
 * The revoke button of a row. A token that no longer works has nothing to revoke; an admin
 * sees it disabled, with the reason, on an owner's token or a system token.
 */
export function RevokeButton({ me, token, onRevoke }: RevokeButtonProps) {
  const { t } = useTranslation()
  if (token.status !== 'active') return null
  const allowed = canRevoke(me, token)
  return (
    <Button
      variant="outline"
      size="sm"
      aria-label={t('tokens.revoke.row', { name: token.name })}
      disabled={!allowed}
      {...(allowed || !isOwnerLevel(token) ? {} : { title: t('tokens.revoke.ownerOnly') })}
      onClick={() => {
        onRevoke(token)
      }}
    >
      {t('tokens.revoke.action')}
    </Button>
  )
}
