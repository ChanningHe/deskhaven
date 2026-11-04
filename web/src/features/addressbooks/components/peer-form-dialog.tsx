import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  useAddAddressBookPeer,
  useListAddressBookTags,
  useUpdateAddressBookPeer,
} from '@/api/generated/address-books/address-books'
import type { PeerBody, PeerView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { invalidateAddressBooks } from '../invalidate'

type TextField = 'alias' | 'hostname' | 'username' | 'platform' | 'note'
const TEXT_FIELDS: readonly TextField[] = ['alias', 'hostname', 'username', 'platform', 'note']

interface PeerFormDialogProps {
  guid: string
  /** The entry to edit, `'new'` to add one, `null` for closed. */
  peer: PeerView | 'new' | null
  onClose: () => void
}

/** Add or edit one entry. The password is write-only: it is never shown, only replaced. */
export function PeerFormDialog({ guid, peer, onClose }: PeerFormDialogProps) {
  const { t } = useTranslation()
  return (
    <Dialog
      open={peer !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {peer === 'new' || peer === null
              ? t('addressBooks.entries.form.addTitle')
              : t('addressBooks.entries.form.editTitle', { id: peer.id })}
          </DialogTitle>
          <DialogDescription>{t('addressBooks.entries.form.passwordNote')}</DialogDescription>
        </DialogHeader>
        {peer === null ? null : (
          <PeerForm
            key={peer === 'new' ? 'new' : peer.id}
            guid={guid}
            peer={peer === 'new' ? undefined : peer}
            onDone={onClose}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function PeerForm({
  guid,
  peer,
  onDone,
}: {
  guid: string
  peer: PeerView | undefined
  onDone: () => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [entryId, setEntryId] = useState('')
  const [password, setPassword] = useState('')
  const [removePassword, setRemovePassword] = useState(false)
  const tagList = useListAddressBookTags(guid)
  const { draft, changed, set } = useRebasedDraft({
    alias: peer?.alias ?? '',
    hostname: peer?.hostname ?? '',
    username: peer?.username ?? '',
    platform: peer?.platform ?? '',
    note: peer?.note ?? '',
    tags: peer?.tags ?? [],
    force_always_relay: peer?.force_always_relay ?? false,
  })
  const add = useAddAddressBookPeer()
  const update = useUpdateAddressBookPeer()
  const mutation = peer === undefined ? add : update

  const toggleTag = (name: string, on: boolean) => {
    set({ tags: on ? [...draft.tags, name] : draft.tags.filter((tag) => tag !== name) })
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    // Edit: only what the user changed. Add: every field.
    const fields: PeerBody = {}
    for (const key of [...TEXT_FIELDS, 'tags', 'force_always_relay'] as const) {
      if (peer === undefined || changed.includes(key)) Object.assign(fields, { [key]: draft[key] })
    }
    if (password !== '') fields.password = password
    else if (removePassword) fields.password = ''
    const saved =
      peer === undefined
        ? await add.mutateAsync({ guid, data: { ...fields, id: entryId } }).catch(() => undefined)
        : await update.mutateAsync({ guid, id: peer.id, data: fields }).catch(() => undefined)
    if (saved === undefined) return
    void invalidateAddressBooks(queryClient)
    onDone()
  }

  return (
    <form className="grid gap-2" onSubmit={(event) => void submit(event)}>
      <Label htmlFor={`${id}-id`}>{t('addressBooks.entries.form.id')}</Label>
      <Input
        id={`${id}-id`}
        required
        maxLength={128}
        disabled={peer !== undefined}
        value={peer?.id ?? entryId}
        onChange={(e) => {
          setEntryId(e.target.value)
        }}
      />
      {TEXT_FIELDS.map((field) => (
        <div key={field} className="grid gap-2">
          <Label htmlFor={`${id}-${field}`}>{t(`addressBooks.entries.form.${field}`)}</Label>
          <Input
            id={`${id}-${field}`}
            value={draft[field]}
            onChange={(e) => {
              set({ [field]: e.target.value })
            }}
          />
        </div>
      ))}
      <Label htmlFor={`${id}-password`}>{t('addressBooks.entries.form.password')}</Label>
      <Input
        id={`${id}-password`}
        type="password"
        autoComplete="new-password"
        maxLength={256}
        disabled={removePassword}
        placeholder={
          peer?.has_password === true ? t('addressBooks.entries.form.passwordKept') : undefined
        }
        value={password}
        onChange={(e) => {
          setPassword(e.target.value)
        }}
      />
      {peer?.has_password === true ? (
        <Label className="font-normal">
          <Checkbox
            checked={removePassword}
            onCheckedChange={(checked) => {
              setRemovePassword(checked)
              if (checked) setPassword('')
            }}
          />
          {t('addressBooks.entries.form.removePassword')}
        </Label>
      ) : null}
      <Label className="font-normal">
        <Checkbox
          checked={draft.force_always_relay}
          onCheckedChange={(checked) => {
            set({ force_always_relay: checked })
          }}
        />
        {t('addressBooks.entries.form.forceRelay')}
      </Label>
      {tagList.data === undefined || tagList.data.items.length === 0 ? null : (
        <fieldset className="grid gap-1">
          <legend className="text-sm font-medium">{t('addressBooks.entries.form.tags')}</legend>
          {tagList.data.items.map((tag) => (
            <Label key={tag.name} className="font-normal">
              <Checkbox
                checked={draft.tags.includes(tag.name)}
                onCheckedChange={(checked) => {
                  toggleTag(tag.name, checked)
                }}
              />
              <bdi>{tag.name}</bdi>
            </Label>
          ))}
        </fieldset>
      )}
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
      <DialogFooter>
        <Button type="submit" disabled={mutation.isPending}>
          {peer === undefined
            ? t('addressBooks.entries.form.add')
            : t('addressBooks.entries.form.save')}
        </Button>
      </DialogFooter>
    </form>
  )
}
