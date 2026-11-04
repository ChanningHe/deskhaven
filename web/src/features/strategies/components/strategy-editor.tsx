import { useTranslation } from 'react-i18next'

import type { StrategyView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'

import { useStrategyEditor } from '../use-strategy-editor'
import { AdvancedOptions } from './advanced-options'
import { OptionGroups } from './option-groups'
import { StrategyDetails } from './strategy-details'

interface SaveBarProps {
  dirty: boolean
  blocked: boolean
  pending: boolean
  saved: boolean
}

function SaveBar({ dirty, blocked, pending, saved }: SaveBarProps) {
  const { t } = useTranslation()
  return (
    <div className="sticky bottom-0 flex items-center gap-2 border-t bg-background py-3">
      <Button type="submit" size="sm" disabled={!dirty || blocked || pending}>
        {t('strategies.editor.save')}
      </Button>
      <p role="status" className="text-xs text-muted-foreground">
        {saved && !dirty ? t('strategies.editor.saved') : null}
      </p>
    </div>
  )
}

/** Name, note and options of a strategy, saved together as one merge patch. */
export function StrategyEditor({ strategy }: { strategy: StrategyView }) {
  const { t } = useTranslation()
  const editor = useStrategyEditor(strategy)
  const { catalog, details, options, mutation, problems } = editor
  const editable = strategy.managed_by === 'ui'
  return (
    <form className="grid gap-4" onSubmit={(event) => void editor.submit(event)}>
      <StrategyDetails draft={details.draft} onChange={details.set} disabled={!editable} />
      {catalog.error ? (
        <ErrorAlert error={catalog.error} onRetry={() => void catalog.refetch()} />
      ) : null}
      <OptionGroups
        options={catalog.data?.options ?? []}
        draft={options.draft}
        saved={new Set(strategy.secret_options)}
        problems={problems}
        onChange={editor.setOption}
        disabled={!editable}
      />
      <AdvancedOptions
        entries={editor.free}
        known={new Set(editor.known.keys())}
        problems={problems}
        onChange={editor.setOption}
        disabled={!editable}
      />
      {editor.conflicts ? (
        <p role="status" className="text-xs text-warning">
          {t('strategies.editor.conflict')}
        </p>
      ) : null}
      {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
      {editable ? (
        <SaveBar
          dirty={editor.dirty}
          blocked={problems.size > 0}
          pending={mutation.isPending}
          saved={mutation.isSuccess}
        />
      ) : null}
    </form>
  )
}
