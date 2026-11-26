import { useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { deleteStrategy, getGetStrategyQueryKey } from '@/api/generated/strategies/strategies'
import type { StrategyView } from '@/api/generated/model'
import { DangerZone } from '@/components/common/danger-zone'

import { invalidateStrategies } from '../invalidate'

interface StrategyDangerZoneProps {
  strategy: StrategyView
  onDeleted: () => void
}

export function StrategyDangerZone({ strategy, onDeleted }: StrategyDangerZoneProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  return (
    <DangerZone
      label={t('strategies.delete.action')}
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
  )
}
