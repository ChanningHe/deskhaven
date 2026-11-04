import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteAddressBook,
  getGetAddressBookQueryKey,
  useUpdateAddressBook,
} from '@/api/generated/address-books/address-books'
import type { BookView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { invalidateAddressBooks } from '../invalidate'

/** Name and note of a book, and delete (confirmed by typing the name). */
export function BookDetails({ book, onDeleted }: { book: BookView; onDeleted: () => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const { draft, changed, conflicts, set, reset } = useRebasedDraft({
    name: book.name,
    note: book.note,
  })
  const mutation = useUpdateAddressBook()
  const dirty = changed.length > 0
  const detailKey = getGetAddressBookQueryKey(book.guid)

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    // Only what the user edited: an unrelated field must not write a stale value back.
    const sent = {
      ...(changed.includes('name') ? { name: draft.name } : {}),
      ...(changed.includes('note') ? { note: draft.note } : {}),
    }
    const saved = await mutation.mutateAsync({ guid: book.guid, data: sent }).catch(() => undefined)
    if (saved === undefined) return
    // The response is the newest copy: a refetch still in flight must not overwrite it.
    await queryClient.cancelQueries({ queryKey: detailKey })
    queryClient.setQueryData<BookView>(detailKey, saved)
    reset({ name: saved.name, note: saved.note }, sent)
    void invalidateAddressBooks(queryClient)
  }

  // The confirm dialog is a sibling of the form, never inside it: React bubbles its submit
  // through the portal, which would also save the form.
  return (
    <>
      <form className="grid max-w-md gap-2" onSubmit={(event) => void submit(event)}>
        <Label htmlFor={`${id}-name`}>{t('addressBooks.fields.name')}</Label>
        <Input
          id={`${id}-name`}
          required
          maxLength={64}
          value={draft.name}
          onChange={(e) => {
            set({ name: e.target.value })
          }}
        />
        <Label htmlFor={`${id}-note`}>{t('addressBooks.fields.note')}</Label>
        <Input
          id={`${id}-note`}
          value={draft.note}
          onChange={(e) => {
            set({ note: e.target.value })
          }}
        />
        {conflicts.length > 0 ? (
          <p role="status" className="text-xs text-warning">
            {t('addressBooks.drawer.conflict')}
          </p>
        ) : null}
        {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
        <div className="flex gap-2">
          <Button type="submit" size="sm" disabled={!dirty || mutation.isPending}>
            {t('addressBooks.drawer.save')}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="destructive"
            onClick={() => {
              setConfirming(true)
            }}
          >
            {t('addressBooks.drawer.delete')}
          </Button>
        </div>
      </form>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('addressBooks.confirmDelete.title')}
        description={t('addressBooks.confirmDelete.description', {
          name: book.name,
          count: book.peer_count,
        })}
        confirmLabel={t('addressBooks.confirmDelete.confirm')}
        confirmText={book.name}
        onConfirm={async () => {
          await deleteAddressBook(book.guid)
          onDeleted()
          // Nothing to refetch: the book is gone.
          queryClient.removeQueries({ queryKey: detailKey })
          await invalidateAddressBooks(queryClient)
        }}
      />
    </>
  )
}
