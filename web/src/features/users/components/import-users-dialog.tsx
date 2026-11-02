import { useQueryClient } from '@tanstack/react-query'
import { Upload } from 'lucide-react'
import { useId, useState } from 'react'
import type { ChangeEvent, SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useImportUsers } from '@/api/generated/users/users'
import { ErrorAlert } from '@/components/common/error-alert'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'

import { invalidateAccounts } from '../invalidate'
import { SECRET_MUTATION } from '../secret-mutation'
import { importRowErrors } from '../import-errors'
import { ImportRowErrors } from './import-row-errors'

/** Largest CSV the server takes (`MAX_CSV_BYTES`). */
const MAX_CSV_BYTES = 64 * 1024

/** "Import CSV": all or nothing; created accounts' temporary passwords are shown once. */
export function ImportUsersButton() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const id = useId()
  const [open, setOpen] = useState(false)
  const [csv, setCsv] = useState('')
  const mutation = useImportUsers(SECRET_MUTATION)
  const rows = importRowErrors(mutation.error)
  const created = mutation.data?.created
  const secret = created?.map((row) => `${row.user.username}\t${row.temporary_password}`).join('\n')

  const loadFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (file !== undefined) setCsv(await file.text())
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const result = await mutation.mutateAsync({ data: { csv } }).catch(() => undefined)
    if (result === undefined) return
    setOpen(false)
    setCsv('')
    void invalidateAccounts(queryClient)
  }

  return (
    <>
      <Button
        variant="outline"
        onClick={() => {
          mutation.reset()
          setOpen(true)
        }}
      >
        <Upload aria-hidden />
        {t('users.import.action')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          if (!next && !mutation.isPending) mutation.reset()
        }}
      >
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t('users.import.title')}</DialogTitle>
            <DialogDescription>{t('users.import.description')}</DialogDescription>
          </DialogHeader>
          <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
            <Label htmlFor={`${id}-file`}>{t('users.import.file')}</Label>
            <input
              id={`${id}-file`}
              type="file"
              accept=".csv,text/csv"
              className="text-sm"
              onChange={(event) => void loadFile(event)}
            />
            <Label htmlFor={`${id}-csv`}>{t('users.import.text')}</Label>
            <textarea
              id={`${id}-csv`}
              required
              rows={8}
              spellCheck={false}
              maxLength={MAX_CSV_BYTES}
              placeholder="username,display_name,email,role,note"
              value={csv}
              onChange={(event) => {
                setCsv(event.target.value)
              }}
              className="w-full rounded-lg border border-input bg-transparent px-2.5 py-2 font-mono text-xs outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
            />
            {rows.length > 0 ? <ImportRowErrors rows={rows} /> : null}
            {mutation.error && rows.length === 0 ? <ErrorAlert error={mutation.error} /> : null}
            <DialogFooter>
              <Button type="submit" disabled={mutation.isPending} aria-busy={mutation.isPending}>
                {t('users.import.submit')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <SecretRevealDialog
        open={secret !== undefined}
        title={t('users.import.secretTitle', { count: created?.length ?? 0 })}
        description={t('users.import.secretDescription')}
        secret={secret ?? ''}
        onDone={() => {
          mutation.reset()
        }}
      />
    </>
  )
}
