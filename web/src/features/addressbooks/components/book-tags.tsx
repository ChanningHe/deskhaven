import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { CSSProperties } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteAddressBookTags,
  useAddAddressBookTag,
  useListAddressBookTags,
} from '@/api/generated/address-books/address-books'
import type { TagView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { invalidateAddressBooks } from '../invalidate'
import { argbToHex, DEFAULT_TAG_HEX, hexToArgb } from '../tag-color'
import { EditTag } from './edit-tag'

/** The tags of one book: add, rename / recolour, delete (they leave the entries too). */
export function BookTags({ guid }: { guid: string }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const tags = useListAddressBookTags(guid)
  const [editing, setEditing] = useState<TagView | null>(null)
  const [removing, setRemoving] = useState<TagView | null>(null)
  const [confirming, setConfirming] = useState(false)
  // A rename that was kept although the recolour after it failed.
  const [renamed, setRenamed] = useState<string | null>(null)

  return (
    <section className="grid gap-3">
      <AddTag guid={guid} />
      {renamed === null ? null : (
        <p role="alert" className="text-sm text-warning">
          {t('addressBooks.tags.renameKept', { name: renamed })}
        </p>
      )}
      {tags.error ? <ErrorAlert error={tags.error} onRetry={() => void tags.refetch()} /> : null}
      {tags.data?.items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('addressBooks.tags.empty')}</p>
      ) : null}
      <ul className="grid gap-1 text-sm">
        {tags.data?.items.map((tag) =>
          editing?.name === tag.name ? (
            <li key={tag.name}>
              <EditTag
                guid={guid}
                tag={tag}
                onDone={() => {
                  setEditing(null)
                }}
                onRenamed={setRenamed}
              />
            </li>
          ) : (
            <li key={tag.name} className="flex items-center justify-between gap-2">
              <span className="flex items-center gap-2">
                <span
                  aria-hidden
                  className="size-4 rounded-full border bg-(--tag-color)"
                  style={{ '--tag-color': argbToHex(tag.color) } as CSSProperties}
                />
                <bdi>{tag.name}</bdi>
              </span>
              <span className="flex gap-1">
                <Button
                  size="xs"
                  variant="ghost"
                  aria-label={t('addressBooks.tags.edit', { name: tag.name })}
                  onClick={() => {
                    setEditing(tag)
                  }}
                >
                  {t('addressBooks.tags.editShort')}
                </Button>
                <Button
                  size="xs"
                  variant="ghost"
                  aria-label={t('addressBooks.tags.remove', { name: tag.name })}
                  onClick={() => {
                    setRemoving(tag)
                    setConfirming(true)
                  }}
                >
                  {t('addressBooks.tags.removeShort')}
                </Button>
              </span>
            </li>
          ),
        )}
      </ul>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('addressBooks.tags.confirmTitle')}
        description={t('addressBooks.tags.confirmDescription', { name: removing?.name ?? '' })}
        confirmLabel={t('addressBooks.tags.removeShort')}
        onConfirm={async () => {
          if (removing === null) return
          await deleteAddressBookTags(guid, { names: [removing.name] })
          await invalidateAddressBooks(queryClient)
        }}
      />
    </section>
  )
}

function AddTag({ guid }: { guid: string }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [hex, setHex] = useState(DEFAULT_TAG_HEX)
  const mutation = useAddAddressBookTag()

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const color = hexToArgb(hex)
    const added = await mutation
      .mutateAsync({ guid, data: { name, ...(color === undefined ? {} : { color }) } })
      .catch(() => undefined)
    if (added === undefined) return
    setName('')
    void invalidateAddressBooks(queryClient)
  }

  return (
    <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => void submit(event)}>
      <div className="grid gap-1">
        <Label htmlFor={`${id}-name`}>{t('addressBooks.tags.name')}</Label>
        <Input
          id={`${id}-name`}
          required
          maxLength={64}
          value={name}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`${id}-color`}>{t('addressBooks.tags.color')}</Label>
        <Input
          id={`${id}-color`}
          type="color"
          className="w-14 p-1"
          value={hex}
          onChange={(e) => {
            setHex(e.target.value)
          }}
        />
      </div>
      <Button type="submit" size="sm" disabled={mutation.isPending}>
        {t('addressBooks.tags.add')}
      </Button>
      {mutation.error ? <ErrorAlert className="w-full" error={mutation.error} /> : null}
    </form>
  )
}
