import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/common/confirm-dialog'
import type { ConfirmDialogProps } from '@/components/common/confirm-dialog'
import { Button } from '@/components/ui/button'

type DangerZoneProps = Pick<
  ConfirmDialogProps,
  'title' | 'description' | 'confirmLabel' | 'confirmText' | 'onConfirm'
> & {
  /** The button that opens the confirmation. */
  label: string
}

/**
 * The irreversible action of a drawer, kept last and apart from everyday actions. Render it
 * outside any form: React bubbles the dialog's submit through the portal.
 */
export function DangerZone({ label, ...confirm }: DangerZoneProps) {
  const { t } = useTranslation()
  const headingId = useId()
  const [confirming, setConfirming] = useState(false)
  return (
    <section
      aria-labelledby={headingId}
      className="grid gap-3 rounded-lg border border-destructive/30 p-3"
    >
      <div className="grid gap-1">
        <h3 id={headingId} className="text-sm font-medium text-destructive">
          {t('common.dangerZone.title')}
        </h3>
        <p className="text-xs text-muted-foreground">{t('common.dangerZone.description')}</p>
      </div>
      <Button
        type="button"
        size="sm"
        variant="destructive"
        className="w-fit"
        onClick={() => {
          setConfirming(true)
        }}
      >
        {label}
      </Button>
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} {...confirm} />
    </section>
  )
}
