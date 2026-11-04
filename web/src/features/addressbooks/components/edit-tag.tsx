import { useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useUpdateAddressBookTag } from '@/api/generated/address-books/address-books'
import type { TagView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { invalidateAddressBooks } from '../invalidate'
import { argbToHex, hexToArgb } from '../tag-color'

/** Rename and / or recolour one tag, inline. */
export function EditTag({ guid, tag, onDone }: { guid: string; tag: TagView; onDone: () => void }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [name, setName] = useState(tag.name)
  const [hex, setHex] = useState(argbToHex(tag.color))
  const mutation = useUpdateAddressBookTag()

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const color = hexToArgb(hex)
    const data = {
      ...(name === tag.name ? {} : { name }),
      ...(hex === argbToHex(tag.color) || color === undefined ? {} : { color }),
    }
    if (Object.keys(data).length === 0) {
      onDone()
      return
    }
    const saved = await mutation.mutateAsync({ guid, name: tag.name, data }).catch(() => undefined)
    if (saved === undefined) return
    void invalidateAddressBooks(queryClient)
    onDone()
  }

  return (
    <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => void submit(event)}>
      <Label htmlFor={`${id}-name`} className="sr-only">
        {t('addressBooks.tags.name')}
      </Label>
      <Input
        id={`${id}-name`}
        required
        maxLength={64}
        value={name}
        onChange={(e) => {
          setName(e.target.value)
        }}
      />
      <Label htmlFor={`${id}-color`} className="sr-only">
        {t('addressBooks.tags.color')}
      </Label>
      <Input
        id={`${id}-color`}
        type="color"
        className="w-14 p-1"
        value={hex}
        onChange={(e) => {
          setHex(e.target.value)
        }}
      />
      <Button type="submit" size="sm" disabled={mutation.isPending}>
        {t('addressBooks.tags.save')}
      </Button>
      <Button type="button" size="sm" variant="outline" onClick={onDone}>
        {t('common.cancel')}
      </Button>
      {mutation.error ? <ErrorAlert className="w-full" error={mutation.error} /> : null}
    </form>
  )
}
