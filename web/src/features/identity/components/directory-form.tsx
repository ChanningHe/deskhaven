import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteLdapDirectory,
  getListLdapDirectoriesQueryKey,
  useCreateLdapDirectory,
  useUpdateLdapDirectory,
} from '@/api/generated/identity/identity'
import type { LdapDirectoryView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import {
  directoryDraft,
  directoryPatch,
  EMPTY_DIRECTORY,
  needsPassword,
  newDirectoryBody,
} from '../ldap-form'
import { DirectoryFields } from './directory-fields'
import { FieldProblems } from './form-controls'
import { LdapDiagnostics } from './ldap-diagnostics'
import type { TestTarget } from './ldap-diagnostics'

/**
 * The owner's directory form: create or edit, delete, and the tests. Tests run against
 * the saved directory, or against the form as typed while it has unsaved changes.
 */
export function DirectoryForm({ directory }: { directory: LdapDirectoryView | undefined }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const { draft, changed, conflicts, set, reset } = useRebasedDraft(
    directory === undefined ? EMPTY_DIRECTORY : directoryDraft(directory),
  )
  const create = useCreateLdapDirectory(SECRET_MUTATION)
  const update = useUpdateLdapDirectory(SECRET_MUTATION)
  const mutation = directory === undefined ? create : update
  const [confirming, setConfirming] = useState(false)
  const missingPassword = directory !== undefined && needsPassword(draft, changed)
  const target: TestTarget =
    directory !== undefined && changed.length === 0
      ? { directory_id: directory.id }
      : { directory: newDirectoryBody(draft) }
  const listKey = getListLdapDirectoriesQueryKey()

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const sent = { ...draft }
    const saved =
      directory === undefined
        ? await create.mutateAsync({ data: newDirectoryBody(draft) }).catch(() => undefined)
        : await update
            .mutateAsync({ id: directory.id, data: directoryPatch(draft, changed) })
            .catch(() => undefined)
    if (saved === undefined) return
    reset(directoryDraft(saved), sent)
    await queryClient.invalidateQueries({ queryKey: listKey })
  }

  // The confirm dialog is a sibling of the form: React bubbles its submit through the portal.
  return (
    <div className="grid gap-6">
      <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
        <DirectoryFields
          draft={draft}
          set={set}
          stored={
            directory === undefined ? undefined : { hasPassword: directory.has_bind_password }
          }
          needsPassword={missingPassword}
          disabled={mutation.isPending}
        />
        {conflicts.length > 0 ? (
          <p role="status" className="text-xs text-warning">
            {t('identity.ldap.conflict')}
          </p>
        ) : null}
        {mutation.error ? (
          <div className="grid gap-1">
            <ErrorAlert error={mutation.error} />
            <FieldProblems error={mutation.error} />
          </div>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button
            type="submit"
            disabled={mutation.isPending || missingPassword || changed.length === 0}
          >
            {directory === undefined ? t('identity.ldap.create') : t('identity.ldap.save')}
          </Button>
          {directory === undefined ? null : (
            <Button
              type="button"
              variant="destructive"
              onClick={() => {
                setConfirming(true)
              }}
            >
              {t('identity.ldap.delete')}
            </Button>
          )}
        </div>
      </form>
      <LdapDiagnostics target={target} />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('identity.ldap.confirmDelete.title')}
        description={t('identity.ldap.confirmDelete.description', { name: directory?.name ?? '' })}
        confirmLabel={t('identity.ldap.confirmDelete.confirm')}
        confirmText={directory?.name ?? ''}
        onConfirm={async () => {
          if (directory === undefined) return
          await deleteLdapDirectory(directory.id)
          await queryClient.invalidateQueries({ queryKey: listKey })
        }}
      />
    </div>
  )
}
