import { useNavigate, useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { useGetStrategy } from '@/api/generated/strategies/strategies'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'
import { httpStatusOf } from '@/lib/http-error'

import { STRATEGY_TABS } from '../strategies-search'
import type { StrategiesSearch, StrategyTab } from '../strategies-search'
import { optionCount } from './strategy-columns'
import { DeleteStrategyButton } from './delete-strategy-button'
import { StrategyAssignments } from './strategy-assignments'
import { StrategyEditor } from './strategy-editor'
import { StrategyPreview } from './strategy-preview'

const HTTP_NOT_FOUND = 404

interface StrategyDrawerProps {
  strategyId: number | undefined
  onClose: () => void
}

/** Side drawer of one strategy: its options, its targets and a device preview. */
export function StrategyDrawer({ strategyId, onClose }: StrategyDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={strategyId !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-3xl">
        {strategyId === undefined ? null : <DrawerBody id={strategyId} onDeleted={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({ id, onDeleted }: { id: number; onDeleted: () => void }) {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/strategies' })
  const navigate = useNavigate({ from: '/strategies' })
  const query = useGetStrategy(id)
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('strategies.drawer.title')}</SheetTitle>
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <SheetDescription>{t('common.loading')}</SheetDescription>
        )}
      </SheetHeader>
    )
  }
  const strategy = query.data
  const editable = strategy.managed_by === 'ui'
  // A later refetch failed: what follows is the last good copy (or the strategy is gone).
  const failure = query.isError ? query.error : undefined
  const gone = httpStatusOf(failure) === HTTP_NOT_FOUND
  const tab: StrategyTab = search.tab ?? 'options'
  return (
    <>
      <SheetHeader>
        <SheetTitle>{strategy.name}</SheetTitle>
        <SheetDescription>
          {t('strategies.drawer.summary', {
            options: optionCount(strategy),
            assignments: strategy.assignment_count,
          })}
        </SheetDescription>
        <div className="flex items-center gap-2 pt-2 text-xs text-muted-foreground">
          {editable ? (
            <DeleteStrategyButton strategy={strategy} onDeleted={onDeleted} />
          ) : (
            <>
              <SourceBadge source={{ kind: 'file' }} />
              {t('strategies.drawer.readOnly')}
            </>
          )}
        </div>
      </SheetHeader>
      {gone ? (
        <p role="alert" className="mx-4 rounded-lg bg-muted px-3 py-2 text-sm">
          {t('strategies.drawer.gone')}
        </p>
      ) : (
        <div className="grid gap-4 px-4 pb-6">
          {failure === undefined ? null : (
            <ErrorAlert
              error={failure}
              title={t('strategies.drawer.stale')}
              onRetry={() => void query.refetch()}
            />
          )}
          <Tabs
            value={tab}
            onValueChange={(next: StrategyTab) => {
              void navigate({
                search: (previous: StrategiesSearch) => ({
                  ...previous,
                  tab: next === 'options' ? undefined : next,
                }),
                replace: true,
              })
            }}
          >
            <TabsList aria-label={t('strategies.drawer.tabsLabel')}>
              {STRATEGY_TABS.map((name) => (
                <TabsTab key={name} value={name}>
                  {t(`strategies.drawer.tabs.${name}`)}
                </TabsTab>
              ))}
            </TabsList>
            {/* Kept mounted: switching tabs must not drop unsaved edits. */}
            <TabsPanel value="options" keepMounted>
              <StrategyEditor key={strategy.id} strategy={strategy} />
            </TabsPanel>
            <TabsPanel value="assignments">
              <StrategyAssignments strategy={strategy} />
            </TabsPanel>
            <TabsPanel value="preview">
              <StrategyPreview />
            </TabsPanel>
          </Tabs>
        </div>
      )}
    </>
  )
}
