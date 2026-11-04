import { useQueryClient } from '@tanstack/react-query'
import { X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { unassignStrategy, useListStrategyAssignments } from '@/api/generated/strategies/strategies'
import type { AssignmentView, StrategyView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'

import { invalidateStrategies } from '../invalidate'
import { targetLabel } from '../target-label'
import { AddAssignment } from './add-assignment'

/** The targets of a strategy: list, remove, add. */
export function StrategyAssignments({ strategy }: { strategy: StrategyView }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const query = useListStrategyAssignments(strategy.id)
  // `removing` outlives `confirming`: the dialog keeps its texts while it animates closed.
  const [removing, setRemoving] = useState<AssignmentView | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)
  const editable = strategy.managed_by === 'ui'
  const items = query.data?.items ?? []
  return (
    <section className="grid gap-4">
      <p className="text-xs text-muted-foreground">{t('strategies.assignments.priority')}</p>
      {query.error ? <ErrorAlert error={query.error} onRetry={() => void query.refetch()} /> : null}
      {query.data !== undefined && items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('strategies.assignments.none')}</p>
      ) : null}
      <ul aria-label={t('strategies.assignments.title')} className="grid gap-1">
        {items.map((a) => {
          const label = targetLabel(a, t)
          return (
            <li
              key={`${a.target_type}-${String(a.target_id)}`}
              className="flex items-center justify-between gap-2 rounded-lg border px-3 py-1.5 text-sm"
            >
              <span>{label}</span>
              {editable ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t('strategies.assignments.remove', { target: label })}
                  onClick={() => {
                    setRemoving(a)
                    setConfirming(true)
                  }}
                >
                  <X aria-hidden />
                </Button>
              ) : null}
            </li>
          )
        })}
      </ul>
      {editable ? <AddAssignment strategyId={strategy.id} /> : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('strategies.assignments.confirmTitle')}
        description={t('strategies.assignments.confirmDescription', {
          target: removing === undefined ? '' : targetLabel(removing, t),
          name: strategy.name,
        })}
        confirmLabel={t('strategies.assignments.confirm')}
        onConfirm={async () => {
          if (removing === undefined) return
          const params = {
            target_type: removing.target_type,
            ...(removing.target_id == null ? {} : { target_id: removing.target_id }),
          }
          await unassignStrategy(strategy.id, params)
          await invalidateStrategies(queryClient)
        }}
      />
    </section>
  )
}
