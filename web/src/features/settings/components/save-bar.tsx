import { useTranslation } from 'react-i18next'

import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'

interface SaveBarProps {
  count: number
  /** How many settings the last save wrote; shown until the next edit. */
  savedCount: number | undefined
  saving: boolean
  /** Hold back the save: some edit is invalid in the browser already. */
  blocked: boolean
  /** The save's failure, unless something else (conflict, field reasons) explains it. */
  error: unknown
  onSave: () => void
  onDiscard: () => void
}

/** The page's save area: what is pending, Save and Discard, the last outcome. */
export function SaveBar({
  count,
  savedCount,
  saving,
  blocked,
  error,
  onSave,
  onDiscard,
}: SaveBarProps) {
  const { t } = useTranslation()
  return (
    <div className="sticky bottom-0 -mx-4 grid gap-2 border-t bg-background/95 px-4 py-3 backdrop-blur">
      {error == null ? null : <ErrorAlert error={error} title={t('settings.save.failed')} />}
      <div className="flex flex-wrap items-center gap-3">
        <p role="status" className="text-sm text-muted-foreground">
          {count > 0
            ? t('settings.save.pending', { count })
            : savedCount === undefined
              ? t('settings.save.nothing')
              : t('settings.save.saved', { count: savedCount })}
        </p>
        <div className="ml-auto flex gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={count === 0 || saving}
            onClick={onDiscard}
          >
            {t('settings.save.discard')}
          </Button>
          <Button
            type="button"
            disabled={count === 0 || saving || blocked}
            aria-busy={saving}
            onClick={onSave}
          >
            {t('settings.save.save')}
          </Button>
        </div>
      </div>
    </div>
  )
}
