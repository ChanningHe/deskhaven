import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { deleteStrategy, getGetStrategyQueryKey } from '@/api/generated/strategies/strategies'
import type { StrategyView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { Button } from '@/components/ui/button'

import { invalidateStrategies } from '../invalidate'

interface DeleteStrategyButtonProps {
  strategy: StrategyView
  onDeleted: () => void
}

/** Deletes the strategy after the admin types its name. */
export function DeleteStrategyButton({ strategy, onDeleted }: DeleteStrategyButtonProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="destructive"
        onClick={() => {
          setConfirming(true)
        }}
      >
        {t('strategies.delete.action')}
      </Button>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('strategies.delete.title')}
        description={t('strategies.delete.description', {
          name: strategy.name,
          count: strategy.assignment_count,
        })}
        confirmLabel={t('strategies.delete.confirm')}
        confirmText={strategy.name}
        onConfirm={async () => {
          await deleteStrategy(strategy.id)
          onDeleted()
          // Nothing to refetch: the strategy is gone.
          queryClient.removeQueries({ queryKey: getGetStrategyQueryKey(strategy.id) })
          await invalidateStrategies(queryClient)
        }}
      />
    </>
  )
}
