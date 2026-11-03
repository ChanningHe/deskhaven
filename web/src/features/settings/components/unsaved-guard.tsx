import { useBlocker } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/common/confirm-dialog'

/**
 * Stops leaving the page (in-app navigation, closing or reloading the tab) while there are
 * unsaved edits: the browser asks on its own, in-app navigation gets a confirmation.
 */
export function UnsavedGuard({ dirty }: { dirty: boolean }) {
  const { t } = useTranslation()
  const blocker = useBlocker({
    shouldBlockFn: () => dirty,
    enableBeforeUnload: () => dirty,
    withResolver: true,
  })
  return (
    <ConfirmDialog
      open={blocker.status === 'blocked'}
      onOpenChange={(open) => {
        if (!open) blocker.reset?.()
      }}
      title={t('settings.leave.title')}
      description={t('settings.leave.description')}
      confirmLabel={t('settings.leave.confirm')}
      onConfirm={() => {
        blocker.proceed?.()
      }}
    />
  )
}
