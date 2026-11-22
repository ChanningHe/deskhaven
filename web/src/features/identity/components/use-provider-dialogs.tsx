import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteOidcProvider,
  getListOidcProviderConfigsQueryKey,
} from '@/api/generated/identity/identity'
import type { OidcProviderView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'

import { ProviderDialog } from './provider-dialog'
import type { ProviderTarget } from './provider-dialog'

/**
 * The OIDC section's dialogs: create / edit (or view) a provider and confirm deleting one.
 * `providers` is the polled list, so an open edit follows a concurrent change.
 */
export function useProviderDialogs(
  providers: readonly OidcProviderView[] | undefined,
  readOnly: boolean,
) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<ProviderTarget | undefined>(undefined)
  // `deleting` outlives `confirming`: the dialog keeps its texts while it animates closed.
  const [deleting, setDeleting] = useState<OidcProviderView | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)
  const shown =
    editing?.kind === 'stored'
      ? {
          kind: 'stored' as const,
          provider: providers?.find((p) => p.id === editing.provider.id) ?? editing.provider,
        }
      : editing

  const dialogs = (
    <>
      <ProviderDialog
        target={shown}
        readOnly={readOnly}
        onClose={() => {
          setEditing(undefined)
        }}
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('identity.oidc.confirmDelete.title')}
        description={t('identity.oidc.confirmDelete.description', { name: deleting?.name ?? '' })}
        confirmLabel={t('identity.oidc.confirmDelete.confirm')}
        confirmText={deleting?.name ?? ''}
        onConfirm={async () => {
          if (deleting === undefined) return
          await deleteOidcProvider(deleting.id)
          await queryClient.invalidateQueries({ queryKey: getListOidcProviderConfigsQueryKey() })
        }}
      />
    </>
  )
  return {
    edit: setEditing,
    remove: (provider: OidcProviderView) => {
      setDeleting(provider)
      setConfirming(true)
    },
    dialogs,
  }
}
