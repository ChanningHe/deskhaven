import { keepPreviousData } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { SettingView } from '@/api/generated/model'
import { useListSettings } from '@/api/generated/settings/settings'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'

import { categoryLabel, groupByCategory } from '../categories'
import { problemOf } from '../editor'
import { useSettingsEditor } from '../use-settings-editor'
import { ConflictDialog } from './conflict-dialog'
import { HistoryDrawer } from './history-drawer'
import { SaveBar } from './save-bar'
import { SettingRow } from './setting-row'
import { UnsavedGuard } from './unsaved-guard'

/** Settings (05 §3): grouped by category, edited as one batch, with source and history. */
export function SettingsPage() {
  const { t } = useTranslation()
  const query = useListSettings({ query: { placeholderData: keepPreviousData } })
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.settings')} description={t('settings.description')} />
      {query.data === undefined ? (
        query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
        )
      ) : (
        <SettingsForm settings={query.data} />
      )}
    </div>
  )
}

function SettingsForm({ settings }: { settings: readonly SettingView[] }) {
  const { t } = useTranslation()
  const editor = useSettingsEditor(settings)
  const [historyKey, setHistoryKey] = useState<string | undefined>(undefined)
  const draftKeys = Object.keys(editor.drafts)
  const invalid = settings.some((setting) => {
    const draft = editor.drafts[setting.key]
    return draft?.change.kind === 'set' && problemOf(setting, draft.change.editor) !== undefined
  })
  const explained = editor.conflicts.length > 0 || Object.keys(editor.problems).length > 0

  return (
    <>
      <div className="space-y-8">
        {groupByCategory(settings).map((category) => (
          <section key={category.name} aria-labelledby={`category-${category.name}`}>
            <h2 id={`category-${category.name}`} className="text-base font-semibold">
              {categoryLabel(category.name, t)}
            </h2>
            <ul className="divide-y">
              {category.settings.map((setting) => (
                <SettingRow
                  key={setting.key}
                  setting={setting}
                  draft={editor.drafts[setting.key]}
                  serverProblems={editor.problems[setting.key] ?? []}
                  disabled={editor.saving}
                  onEdit={(value) => {
                    editor.edit(setting, value)
                  }}
                  onStageReset={() => {
                    editor.stageReset(setting)
                  }}
                  onUndo={() => {
                    editor.discard(setting.key)
                  }}
                  onHistory={() => {
                    setHistoryKey(setting.key)
                  }}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
      <SaveBar
        count={draftKeys.length}
        savedCount={editor.savedCount}
        saving={editor.saving}
        blocked={invalid}
        error={explained ? undefined : editor.error}
        onSave={() => void editor.save()}
        onDiscard={editor.discardAll}
      />
      <ConflictDialog
        conflicts={editor.conflicts}
        drafts={editor.drafts}
        busy={editor.saving}
        onReload={() => void editor.reload()}
        onOverwrite={() => void editor.overwrite()}
        onCancel={editor.dismissConflict}
      />
      <HistoryDrawer
        setting={settings.find((setting) => setting.key === historyKey)}
        onClose={() => {
          setHistoryKey(undefined)
        }}
      />
      <UnsavedGuard dirty={draftKeys.length > 0} />
    </>
  )
}
