import { useQueryClient } from '@tanstack/react-query'
import { Plus } from 'lucide-react'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useCreateAddressBook } from '@/api/generated/address-books/address-books'
import type { OwnerRef } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { OwnerPicker } from '@/features/devices/components/owner-picker'

import { invalidateAddressBooks } from '../invalidate'

/** "New address book"; opens the new book's drawer when created. */
export function CreateBookButton({ onCreated }: { onCreated: (guid: string) => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [note, setNote] = useState('')
  const [owner, setOwner] = useState<OwnerRef | null>(null)
  const mutation = useCreateAddressBook()

  const clear = () => {
    setName('')
    setNote('')
    setOwner(null)
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const data = { name, note, ...(owner === null ? {} : { owner_user_id: owner.id }) }
    const created = await mutation.mutateAsync({ data }).catch(() => undefined)
    if (created === undefined) return
    setOpen(false)
    clear()
    void invalidateAddressBooks(queryClient)
    onCreated(created.guid)
  }

  return (
    <>
      <Button
        onClick={() => {
          mutation.reset()
          setOpen(true)
        }}
      >
        <Plus aria-hidden />
        {t('addressBooks.create.action')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          // Cancelled: the next opening starts empty, without the last error.
          if (!next) {
            clear()
            if (!mutation.isPending) mutation.reset()
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('addressBooks.create.title')}</DialogTitle>
          </DialogHeader>
          <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
            <Label htmlFor={`${id}-name`}>{t('addressBooks.fields.name')}</Label>
            <Input
              id={`${id}-name`}
              required
              maxLength={64}
              value={name}
              onChange={(e) => {
                setName(e.target.value)
              }}
            />
            <Label htmlFor={`${id}-note`}>{t('addressBooks.fields.note')}</Label>
            <Input
              id={`${id}-note`}
              value={note}
              onChange={(e) => {
                setNote(e.target.value)
              }}
            />
            <div className="grid gap-1">
              <span className="text-sm font-medium">{t('addressBooks.fields.owner')}</span>
              <OwnerPicker value={owner} onChange={setOwner} />
              <p className="text-xs text-muted-foreground">{t('addressBooks.create.ownerHint')}</p>
            </div>
            {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
            <DialogFooter>
              <Button type="submit" disabled={mutation.isPending}>
                {t('addressBooks.create.submit')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
