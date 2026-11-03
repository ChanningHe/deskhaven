import { keepPreviousData } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { HistoryEntryView, SettingView } from '@/api/generated/model'
import { useGetSettingHistory } from '@/api/generated/settings/settings'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { formatDateTime } from '@/lib/format'

import { displayValue } from '../format'

/** Entries per page of the history. */
const HISTORY_PAGE_SIZE = 20

interface HistoryDrawerProps {
  /** The setting whose history is open; closed when undefined. */
  setting: SettingView | undefined
  onClose: () => void
}

/** Side drawer with the changes of one setting, newest first (secrets: only set / unset). */
export function HistoryDrawer({ setting, onClose }: HistoryDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={setting !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-md">
        {setting === undefined ? null : <HistoryBody key={setting.key} setting={setting} />}
      </SheetContent>
    </Sheet>
  )
}

function HistoryBody({ setting }: { setting: SettingView }) {
  const { t } = useTranslation()
  const [page, setPage] = useState(1)
  const history = useGetSettingHistory(
    setting.key,
    { page, page_size: HISTORY_PAGE_SIZE },
    { query: { placeholderData: keepPreviousData } },
  )
  const pages = Math.max(1, Math.ceil((history.data?.total ?? 0) / HISTORY_PAGE_SIZE))
  return (
    <>
      <SheetHeader>
        <SheetTitle>{t('settings.history.title')}</SheetTitle>
        <SheetDescription className="font-mono break-all">{setting.key}</SheetDescription>
      </SheetHeader>
      <div className="grid gap-4 px-4 pb-6">
        {history.error ? (
          <ErrorAlert error={history.error} onRetry={() => void history.refetch()} />
        ) : null}
        {history.data?.items.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t('settings.history.empty')}</p>
        ) : null}
        <ol className="grid gap-3">
          {history.data?.items.map((entry) => (
            <HistoryItem key={entry.version} setting={setting} entry={entry} />
          ))}
        </ol>
        {pages > 1 ? (
          <nav aria-label={t('settings.history.pagination')} className="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={page <= 1}
              onClick={() => {
                setPage(page - 1)
              }}
            >
              {t('settings.history.previous')}
            </Button>
            <span className="text-xs text-muted-foreground">
              {t('settings.history.page', { page, pages })}
            </span>
            <Button
              size="sm"
              variant="outline"
              disabled={page >= pages}
              onClick={() => {
                setPage(page + 1)
              }}
            >
              {t('settings.history.next')}
            </Button>
          </nav>
        ) : null}
      </div>
    </>
  )
}

function HistoryItem({ setting, entry }: { setting: SettingView; entry: HistoryEntryView }) {
  const { t, i18n } = useTranslation()
  const by = entry.updated_by_username ?? t('settings.history.system')
  const before =
    entry.old_value == null
      ? t('settings.history.unset')
      : displayValue(setting, entry.old_value, t)
  return (
    <li className="rounded-lg border p-3 text-sm" data-version={entry.version}>
      <p className="text-xs text-muted-foreground">
        {t('settings.history.meta', {
          version: entry.version,
          by,
          at: formatDateTime(entry.updated_at, i18n.language),
        })}
      </p>
      <p className="mt-1 break-all">
        {before} → {displayValue(setting, entry.new_value, t)}
        {entry.reset ? (
          <span className="ml-2 text-xs font-medium text-warning">
            {t('settings.history.reset')}
          </span>
        ) : null}
      </p>
      {entry.reason == null ? null : (
        <p className="mt-1 text-xs text-muted-foreground">{entry.reason}</p>
      )}
    </li>
  )
}
