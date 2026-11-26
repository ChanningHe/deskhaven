import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { Step } from './panels'

export function ConfirmPanel({
  deviceId,
  onConfirm,
  leave,
}: {
  deviceId: string
  onConfirm: () => void
  leave?: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <Step
      title={t('remote.confirm.title', { id: deviceId })}
      description={t('remote.confirm.description')}
      footer={
        <>
          <Button onClick={onConfirm}>{t('remote.confirm.connect')}</Button>
          {leave}
        </>
      }
    />
  )
}
