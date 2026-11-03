import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'

import type { SettingView } from '@/api/generated/model'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { formatDateTime } from '@/lib/format'

import type { Draft, Drafts } from '../drafts'
import { fromEditor } from '../editor'
import { displayValue } from '../format'

interface ConflictDialogProps {
  /** The current server state of the keys that changed since they were read. */
  conflicts: readonly SettingView[]
  drafts: Drafts
  busy: boolean
  onReload: () => void
  onOverwrite: () => void
  onCancel: () => void
}

/** What my draft would make the value (secrets: only that they would be replaced). */
function mine(setting: SettingView, draft: Draft | undefined, t: TFunction): string {
  if (draft === undefined) return ''
  if (draft.change.kind === 'reset') return t('settings.conflict.resetToDefault')
  if (setting.schema.type === 'secret') return t('settings.conflict.secretReplaced')
  return displayValue(setting, fromEditor(setting, draft.change.editor), t)
}

function changedBy(setting: SettingView, t: TFunction, language: string): string {
  const { source } = setting
  if (source.kind !== 'database') return t('settings.conflict.changedElsewhere')
  const by = source.by_username ?? t('settings.conflict.someone')
  return t('settings.conflict.changedBy', { by, at: formatDateTime(source.at, language) })
}

/**
 * The answer to a 409: for each key someone else changed first, my value next to the
 * server's, then reload (take theirs), overwrite (keep mine on top of theirs) or go back.
 */
export function ConflictDialog({
  conflicts,
  drafts,
  busy,
  onReload,
  onOverwrite,
  onCancel,
}: ConflictDialogProps) {
  const { t, i18n } = useTranslation()
  return (
    <AlertDialog
      open={conflicts.length > 0}
      onOpenChange={(open) => {
        if (!open && !busy) onCancel()
      }}
    >
      <AlertDialogContent className="sm:max-w-2xl">
        <AlertDialogHeader>
          <AlertDialogTitle>{t('settings.conflict.title')}</AlertDialogTitle>
          <AlertDialogDescription>{t('settings.conflict.description')}</AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="grid max-h-80 gap-3 overflow-y-auto">
          {conflicts.map((setting) => (
            <li
              key={setting.key}
              className="rounded-lg border p-3 text-sm"
              data-conflict={setting.key}
            >
              <p className="font-mono font-medium break-all">{setting.key}</p>
              <dl className="mt-2 grid gap-1 sm:grid-cols-[auto_1fr] sm:gap-x-4">
                <dt className="text-muted-foreground">{t('settings.conflict.mine')}</dt>
                <dd className="break-all">{mine(setting, drafts[setting.key], t)}</dd>
                <dt className="text-muted-foreground">{t('settings.conflict.theirs')}</dt>
                <dd className="break-all">{displayValue(setting, setting.value, t)}</dd>
              </dl>
              <p className="mt-1 text-xs text-muted-foreground">
                {changedBy(setting, t, i18n.language)}
              </p>
            </li>
          ))}
        </ul>
        <AlertDialogFooter>
          <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>
            {t('settings.conflict.back')}
          </Button>
          <Button type="button" variant="outline" disabled={busy} onClick={onReload}>
            {t('settings.conflict.reload')}
          </Button>
          <Button type="button" disabled={busy} onClick={onOverwrite}>
            {t('settings.conflict.overwrite')}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
