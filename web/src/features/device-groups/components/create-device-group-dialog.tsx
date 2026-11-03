import { useQueryClient } from '@tanstack/react-query'
import { Plus } from 'lucide-react'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useCreateDeviceGroup } from '@/api/generated/device-groups/device-groups'
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
import { invalidateDevices } from '@/features/devices/invalidate'

/** "New device group"; opens the new group's drawer when created. */
export function CreateDeviceGroupButton({ onCreated }: { onCreated: (id: number) => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [note, setNote] = useState('')
  const mutation = useCreateDeviceGroup()

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const created = await mutation.mutateAsync({ data: { name, note } }).catch(() => undefined)
    if (created === undefined) return
    setOpen(false)
    setName('')
    setNote('')
    void invalidateDevices(queryClient)
    onCreated(created.id)
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
        {t('deviceGroups.create.action')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          // Cancelled: the next opening starts empty, without the last error.
          if (!next) {
            setName('')
            setNote('')
            if (!mutation.isPending) mutation.reset()
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('deviceGroups.create.title')}</DialogTitle>
          </DialogHeader>
          <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
            <Label htmlFor={`${id}-name`}>{t('deviceGroups.fields.name')}</Label>
            <Input
              id={`${id}-name`}
              required
              maxLength={64}
              value={name}
              onChange={(e) => {
                setName(e.target.value)
              }}
            />
            <Label htmlFor={`${id}-note`}>{t('deviceGroups.fields.note')}</Label>
            <Input
              id={`${id}-note`}
              value={note}
              onChange={(e) => {
                setNote(e.target.value)
              }}
            />
            {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
            <DialogFooter>
              <Button type="submit" disabled={mutation.isPending}>
                {t('deviceGroups.create.submit')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
