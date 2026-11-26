import { useBlocker } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/common/confirm-dialog'

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
      title={t('common.leave.title')}
      description={t('common.leave.description')}
      confirmLabel={t('common.leave.confirm')}
      onConfirm={() => {
        blocker.proceed?.()
      }}
    />
  )
}
