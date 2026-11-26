import { History, RotateCcw, Undo2 } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import type { SettingView } from '@/api/generated/model'
import { SourceBadge } from '@/components/common/source-badge'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import type { Draft } from '../drafts'
import { problemOf, toEditor } from '../editor'
import type { Editor } from '../editor'
import { valueSource } from '../format'
import { settingHelp, settingName } from '../labels'
import { CopyCode } from './copy-code'
import { ProblemText, RangeHint, RiskHint } from './setting-hints'
import { SettingControl } from './setting-control'

interface SettingRowProps {
  setting: SettingView
  draft: Draft | undefined
  /** The text as typed, which the draft may have normalised away. */
  typed: Editor | undefined
  /** The server's reasons for refusing this key's draft. */
  serverProblems: readonly string[]
  disabled: boolean
  onEdit: (editor: Editor) => void
  onStageReset: () => void
  onUndo: () => void
  onHistory: () => void
}

/** The editor value to show: the default for a staged reset, else what was typed, else the setting. */
function shownEditor(
  setting: SettingView,
  draft: Draft | undefined,
  typed: Editor | undefined,
): Editor {
  if (draft?.change.kind === 'reset') return toEditor({ ...setting, value: setting.default })
  if (typed !== undefined) return typed
  if (draft?.change.kind === 'set') return draft.change.editor
  return toEditor(setting)
}

/** One setting: name, key and variable, help, source badge, control, and its actions. */
export function SettingRow({
  setting,
  draft,
  typed,
  serverProblems,
  disabled,
  onEdit,
  onStageReset,
  onUndo,
  onHistory,
}: SettingRowProps) {
  const { t } = useTranslation()
  const id = useId()
  const helpId = `${id}-help`
  const problemId = `${id}-problem`
  const editor = shownEditor(setting, draft, typed)
  const staged = draft?.change.kind === 'reset'
  const problem = draft?.change.kind === 'set' ? problemOf(setting, editor) : undefined
  const invalid = problem !== undefined || serverProblems.length > 0
  const source = setting.source
  const readOnly = setting.pinned || setting.schema.pin_only
  const canReset = source.kind === 'database' && !readOnly && draft === undefined

  return (
    <li
      className={cn(
        'relative grid gap-3 py-4 sm:grid-cols-2 sm:gap-8',
        draft !== undefined &&
          'before:absolute before:inset-y-3 before:-left-4 before:w-0.5 before:rounded-r-full before:bg-info sm:before:-left-6',
      )}
      data-setting={setting.key}
    >
      <div className="min-w-0 space-y-1 sm:pt-1">
        <div className="flex items-start gap-2">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <label htmlFor={id} className="text-sm font-medium break-words">
              {settingName(setting.key, t)}
            </label>
            <SourceBadge source={valueSource(source)} />
            {draft === undefined ? null : (
              <span className="inline-flex items-center gap-1.5 text-xs font-medium text-info before:size-1.5 before:rounded-full before:bg-info">
                {t('settings.row.unsaved')}
              </span>
            )}
          </div>
          <HistoryButton onClick={onHistory} />
        </div>
        <div className="flex flex-wrap items-center gap-x-3 font-mono text-[11px] text-muted-foreground">
          <CopyCode value={setting.key} />
          <CopyCode value={setting.schema.env} />
        </div>
        <p id={helpId} className="text-xs text-muted-foreground">
          {settingHelp(setting, t)}
        </p>
        {setting.pinned ? (
          <PinnedNote source={source} />
        ) : setting.schema.pin_only ? (
          <p className="text-xs text-warning">
            {t('settings.pinned.configOnly', { name: setting.schema.env })}
          </p>
        ) : null}
      </div>
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <SettingControl
            setting={setting}
            id={id}
            editor={editor}
            disabled={disabled || readOnly || staged}
            invalid={invalid}
            describedBy={`${helpId} ${problemId}`}
            onChange={onEdit}
          />
          <RangeHint setting={setting} />
        </div>
        <RiskHint setting={setting} editor={editor} />
        <div id={problemId} className="empty:hidden">
          {problem === undefined ? null : <ProblemText problem={problem} />}
          {serverProblems.map((reason) => (
            <p key={reason} role="alert" className="text-xs text-destructive">
              {t('settings.row.serverRefused', { reason })}
            </p>
          ))}
        </div>
        {staged ? (
          <p className="text-xs text-muted-foreground">{t('settings.row.willReset')}</p>
        ) : null}
        <div className="-ml-1.5 flex flex-wrap gap-1 empty:hidden">
          {canReset ? (
            <Button
              type="button"
              variant="ghost"
              size="xs"
              disabled={disabled}
              onClick={onStageReset}
            >
              <RotateCcw data-icon="inline-start" aria-hidden />
              {t('settings.row.reset')}
            </Button>
          ) : null}
          {draft === undefined ? null : (
            <Button type="button" variant="ghost" size="xs" onClick={onUndo}>
              <Undo2 data-icon="inline-start" aria-hidden />
              {t('settings.row.undo')}
            </Button>
          )}
        </div>
      </div>
    </li>
  )
}

function HistoryButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation()
  const label = t('settings.row.history')
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={label}
            className="-mt-0.5 shrink-0 text-muted-foreground hover:text-foreground"
            onClick={onClick}
          />
        }
      >
        <History aria-hidden />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

function PinnedNote({ source }: { source: SettingView['source'] }) {
  const { t } = useTranslation()
  switch (source.kind) {
    case 'env':
      return (
        <p className="text-xs text-warning">{t('settings.pinned.env', { name: source.var })}</p>
      )
    case 'file':
      return (
        <p className="text-xs text-warning">{t('settings.pinned.file', { path: source.path })}</p>
      )
    default:
      return <p className="text-xs text-warning">{t('settings.pinned.other')}</p>
  }
}
