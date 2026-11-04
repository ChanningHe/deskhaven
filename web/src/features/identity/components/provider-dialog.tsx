import { useQueryClient } from '@tanstack/react-query'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  getListOidcProviderConfigsQueryKey,
  useCreateOidcProvider,
  useUpdateOidcProvider,
} from '@/api/generated/identity/identity'
import type { OidcProviderView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { EMPTY_PROVIDER, newProviderBody, providerDraft, providerPatch } from '../oidc-form'
import { FieldProblems } from './form-controls'
import { ProviderFields } from './provider-fields'

/** What the dialog shows: a new provider, or a stored one (from the polled list). */
export type ProviderTarget = { kind: 'new' } | { kind: 'stored'; provider: OidcProviderView }

interface ProviderDialogProps {
  target: ProviderTarget | undefined
  /** Shown without editing (an admin; writes are the owner's). */
  readOnly: boolean
  onClose: () => void
}

/** Creates or edits an OIDC provider; config-managed ones (and any, for admins) read-only. */
export function ProviderDialog({ target, readOnly, onClose }: ProviderDialogProps) {
  const { t } = useTranslation()
  const title =
    target?.kind === 'stored'
      ? t('identity.oidc.editTitle', { name: target.provider.name })
      : t('identity.oidc.createTitle')
  return (
    <Dialog
      open={target !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        {target === undefined ? null : (
          <ProviderForm
            key={target.kind === 'stored' ? target.provider.id : 'new'}
            target={target}
            viewOnly={readOnly}
            onDone={onClose}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

interface ProviderFormProps {
  target: ProviderTarget
  viewOnly: boolean
  onDone: () => void
}

function ProviderForm({ target, viewOnly, onDone }: ProviderFormProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const stored = target.kind === 'stored' ? target.provider : undefined
  const { draft, changed, conflicts, set } = useRebasedDraft(
    stored === undefined ? EMPTY_PROVIDER : providerDraft(stored),
  )
  const create = useCreateOidcProvider(SECRET_MUTATION)
  const update = useUpdateOidcProvider(SECRET_MUTATION)
  const mutation = stored === undefined ? create : update
  const managed = stored?.managed_by === 'config'
  const readOnly = viewOnly || managed

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const saved =
      stored === undefined
        ? await create.mutateAsync({ data: newProviderBody(draft) }).catch(() => undefined)
        : await update
            .mutateAsync({ id: stored.id, data: providerPatch(draft, changed) })
            .catch(() => undefined)
    if (saved === undefined) return
    await queryClient.invalidateQueries({ queryKey: getListOidcProviderConfigsQueryKey() })
    onDone()
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
      {managed ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <SourceBadge source={{ kind: 'file' }} />
          {t('identity.oidc.readOnly')}
        </p>
      ) : null}
      <ProviderFields
        draft={draft}
        set={set}
        stored={stored === undefined ? undefined : { hasSecret: stored.has_client_secret }}
        disabled={readOnly || mutation.isPending}
      />
      {conflicts.length > 0 ? (
        <p role="status" className="text-xs text-warning">
          {t('identity.oidc.conflict')}
        </p>
      ) : null}
      {mutation.error ? (
        <div className="grid gap-1">
          <ErrorAlert error={mutation.error} />
          <FieldProblems error={mutation.error} />
        </div>
      ) : null}
      {readOnly ? null : (
        <DialogFooter>
          <Button
            type="submit"
            disabled={mutation.isPending || (stored !== undefined && changed.length === 0)}
          >
            {stored === undefined ? t('identity.oidc.create') : t('identity.oidc.save')}
          </Button>
        </DialogFooter>
      )}
    </form>
  )
}
