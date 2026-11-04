import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useImportAddressBookPeers } from '@/api/generated/address-books/address-books'
import { useListDevices } from '@/api/generated/devices/devices'
import type { ImportDevicesView } from '@/api/generated/model'
import { SEARCH_DEBOUNCE_MS } from '@/components/common/data-table'
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
import { useDebouncedValue } from '@/lib/use-debounced-value'

import { invalidateAddressBooks } from '../invalidate'

const DEVICES_PER_PAGE = 20
/** Most devices one import takes (the API refuses more). */
const MAX_IMPORT_DEVICES = 500

interface ImportDevicesDialogProps {
  guid: string
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Pick devices of the inventory (any page, any search) and add them as entries. */
export function ImportDevicesDialog({ guid, open, onOpenChange }: ImportDevicesDialogProps) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t('addressBooks.import.title')}</DialogTitle>
          <DialogDescription>{t('addressBooks.import.description')}</DialogDescription>
        </DialogHeader>
        {/* Mounted only while open: every opening starts with nothing picked. */}
        {open ? (
          <ImportBody
            guid={guid}
            onClose={() => {
              onOpenChange(false)
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

function ImportBody({ guid, onClose }: { guid: string; onClose: () => void }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [q, setQ] = useState('')
  const [page, setPage] = useState(1)
  const [picked, setPicked] = useState<ReadonlySet<number>>(new Set())
  const [result, setResult] = useState<ImportDevicesView | undefined>(undefined)
  const term = useDebouncedValue(q.trim(), SEARCH_DEBOUNCE_MS)
  const devices = useListDevices(
    { page, page_size: DEVICES_PER_PAGE, sort: 'rustdesk_id', ...(term === '' ? {} : { q: term }) },
    { query: { placeholderData: keepPreviousData } },
  )
  const mutation = useImportAddressBookPeers()
  const pages = Math.max(1, Math.ceil((devices.data?.total ?? 0) / DEVICES_PER_PAGE))

  const toggle = (id: number, on: boolean) => {
    const next = new Set(picked)
    if (on) next.add(id)
    else next.delete(id)
    setPicked(next)
  }

  const submit = async () => {
    const done = await mutation
      .mutateAsync({ guid, data: { device_ids: [...picked] } })
      .catch(() => undefined)
    if (done === undefined) return
    setResult(done)
    setPicked(new Set())
    void invalidateAddressBooks(queryClient)
  }

  return (
    <div className="grid gap-3">
      <Input
        aria-label={t('addressBooks.import.search')}
        placeholder={t('addressBooks.import.search')}
        value={q}
        onChange={(e) => {
          setQ(e.target.value)
          setPage(1)
        }}
      />
      {devices.error ? (
        <ErrorAlert error={devices.error} onRetry={() => void devices.refetch()} />
      ) : null}
      {devices.data?.items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('addressBooks.import.empty')}</p>
      ) : null}
      <ul className="grid max-h-72 gap-1 overflow-y-auto text-sm">
        {devices.data?.items.map((device) => (
          <li key={device.id}>
            <Label className="font-normal">
              <Checkbox
                checked={picked.has(device.id)}
                disabled={!picked.has(device.id) && picked.size >= MAX_IMPORT_DEVICES}
                onCheckedChange={(checked) => {
                  toggle(device.id, checked)
                }}
              />
              <bdi className="font-medium">{device.rustdesk_id}</bdi>
              <span className="text-muted-foreground">
                <bdi>{[device.hostname, device.username].filter(Boolean).join(' · ')}</bdi>
              </span>
            </Label>
          </li>
        ))}
      </ul>
      <nav
        aria-label={t('addressBooks.import.pagination')}
        className="flex items-center justify-between gap-2 text-xs text-muted-foreground"
      >
        <Button
          size="xs"
          variant="outline"
          disabled={page <= 1}
          onClick={() => {
            setPage(page - 1)
          }}
        >
          {t('dataTable.previousPage')}
        </Button>
        <span>{t('addressBooks.import.page', { page, pages })}</span>
        <Button
          size="xs"
          variant="outline"
          disabled={page >= pages}
          onClick={() => {
            setPage(page + 1)
          }}
        >
          {t('dataTable.nextPage')}
        </Button>
      </nav>
      {picked.size >= MAX_IMPORT_DEVICES ? (
        <p role="status" className="text-xs text-muted-foreground">
          {t('addressBooks.import.limit', { max: MAX_IMPORT_DEVICES })}
        </p>
      ) : null}
      {result === undefined ? null : (
        <p role="status" className="text-sm">
          {t('addressBooks.import.result', {
            added: result.added.length,
            existing: result.existing.length,
            missing: result.missing.length,
          })}
        </p>
      )}
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          {t('common.close')}
        </Button>
        <Button disabled={picked.size === 0 || mutation.isPending} onClick={() => void submit()}>
          {t('addressBooks.import.submit', { count: picked.size })}
        </Button>
      </DialogFooter>
    </div>
  )
}
