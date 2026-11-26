import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import type { ReactNode } from 'react'
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
import { SectionCard } from '@/components/common/section-card'
import { Button } from '@/components/ui/button'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'
import { useRebasedDraft } from '@/lib/use-rebased-draft'
import type { RebasedDraft } from '@/lib/use-rebased-draft'

import {
  directoryDraft,
  directoryPatch,
  EMPTY_DIRECTORY,
  needsPassword,
  newDirectoryBody,
} from '../ldap-form'
import type { DirectoryDraft } from '../ldap-form'
import { useReportDirty } from '../report-dirty'
import type { DirtyReporter } from '../report-dirty'
import { DirectoryFields } from './directory-fields'
import { FieldProblems } from './form-controls'
import { LdapDiagnostics } from './ldap-diagnostics'
import type { TestTarget } from './ldap-diagnostics'

interface DirectoryFormProps {
  directory: LdapDirectoryView | undefined
  status: ReactNode
  alert: ReactNode
  onDirtyChange: DirtyReporter
}

export function DirectoryForm({ directory, status, alert, onDirtyChange }: DirectoryFormProps) {
  const { t } = useTranslation()
  const form = useRebasedDraft(
    directory === undefined ? EMPTY_DIRECTORY : directoryDraft(directory),
  )
  const { draft, changed, conflicts, set } = form
  const { mutation, save } = useSaveDirectory(directory, form)
  useReportDirty(changed.length > 0, onDirtyChange)
  const [confirming, setConfirming] = useState(false)
  const missingPassword = directory !== undefined && needsPassword(draft, changed)
  const target: TestTarget =
    directory !== undefined && changed.length === 0
      ? { directory_id: directory.id }
      : { directory: newDirectoryBody(draft) }

  // The confirm dialog is a sibling of the form: React bubbles its submit through the portal.
  return (
    <>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <SectionCard
          title={t('identity.ldap.title')}
          description={t('identity.ldap.description')}
          status={status}
          footer={
            <DirectoryFooter
              isNew={directory === undefined}
              error={mutation.error}
              conflict={conflicts.length > 0}
              canSave={!mutation.isPending && !missingPassword && changed.length > 0}
              onDelete={() => {
                setConfirming(true)
              }}
            />
          }
        >
          <div className="grid gap-6">
            {alert}
            <DirectoryFields
              draft={draft}
              set={set}
              stored={
                directory === undefined ? undefined : { hasPassword: directory.has_bind_password }
              }
              needsPassword={missingPassword}
              disabled={mutation.isPending}
            />
          </div>
        </SectionCard>
      </form>
      <LdapDiagnostics target={target} />
      {directory === undefined ? null : (
        <DeleteDialog directory={directory} open={confirming} onOpenChange={setConfirming} />
      )}
    </>
  )
}

function useSaveDirectory(
  directory: LdapDirectoryView | undefined,
  { draft, changed, reset }: RebasedDraft<DirectoryDraft>,
) {
  const queryClient = useQueryClient()
  const create = useCreateLdapDirectory(SECRET_MUTATION)
  const update = useUpdateLdapDirectory(SECRET_MUTATION)
  const save = async () => {
    const sent = { ...draft }
    const saved =
      directory === undefined
        ? await create.mutateAsync({ data: newDirectoryBody(draft) }).catch(() => undefined)
        : await update
            .mutateAsync({ id: directory.id, data: directoryPatch(draft, changed) })
            .catch(() => undefined)
    if (saved === undefined) return
    reset(directoryDraft(saved), sent)
    await queryClient.invalidateQueries({ queryKey: getListLdapDirectoriesQueryKey() })
  }
  return { mutation: directory === undefined ? create : update, save }
}

interface DirectoryFooterProps {
  isNew: boolean
  error: unknown
  conflict: boolean
  canSave: boolean
  onDelete: () => void
}

function DirectoryFooter({ isNew, error, conflict, canSave, onDelete }: DirectoryFooterProps) {
  const { t } = useTranslation()
  return (
    <>
      {conflict ? (
        <p role="status" className="w-full text-xs text-warning">
          {t('identity.ldap.conflict')}
        </p>
      ) : null}
      {error ? (
        <div className="grid w-full gap-1">
          <ErrorAlert error={error} />
          <FieldProblems error={error} />
        </div>
      ) : null}
      <Button type="submit" disabled={!canSave}>
        {isNew ? t('identity.ldap.create') : t('identity.ldap.save')}
      </Button>
      {isNew ? null : (
        <Button type="button" variant="destructive" onClick={onDelete}>
          {t('identity.ldap.delete')}
        </Button>
      )}
    </>
  )
}

interface DeleteDialogProps {
  directory: LdapDirectoryView
  open: boolean
  onOpenChange: (open: boolean) => void
}

function DeleteDialog({ directory, open, onOpenChange }: DeleteDialogProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t('identity.ldap.confirmDelete.title')}
      description={t('identity.ldap.confirmDelete.description', { name: directory.name })}
      confirmLabel={t('identity.ldap.confirmDelete.confirm')}
      confirmText={directory.name}
      onConfirm={async () => {
        await deleteLdapDirectory(directory.id)
        await queryClient.invalidateQueries({ queryKey: getListLdapDirectoriesQueryKey() })
      }}
    />
  )
}
