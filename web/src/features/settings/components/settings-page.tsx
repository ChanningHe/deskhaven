import { keepPreviousData } from '@tanstack/react-query'
import { useState } from 'react'
import type { RefCallback } from 'react'
import { useTranslation } from 'react-i18next'

import type { SettingView } from '@/api/generated/model'
import { useListSettings } from '@/api/generated/settings/settings'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'
import { SectionCard } from '@/components/common/section-card'
import { UnsavedGuard } from '@/components/common/unsaved-guard'

import { categoryLabel, groupByCategory, unsavedCount } from '../categories'
import type { Category } from '../categories'
import { problemOf } from '../editor'
import { useCategoryJump } from '../use-category-jump'
import { useSettingsEditor } from '../use-settings-editor'
import { CategoryNav } from './category-nav'
import { ConflictDialog } from './conflict-dialog'
import { HistoryDrawer } from './history-drawer'
import { SaveBar } from './save-bar'
import { SettingRow } from './setting-row'

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

type SettingsEditor = ReturnType<typeof useSettingsEditor>

function SettingsForm({ settings }: { settings: readonly SettingView[] }) {
  const { t } = useTranslation()
  const editor = useSettingsEditor(settings)
  const [historyKey, setHistoryKey] = useState<string | undefined>(undefined)
  const { register, jump } = useCategoryJump()
  const categories = groupByCategory(settings)

  return (
    <>
      {editor.droppedPinned.length === 0 ? null : (
        <p
          role="status"
          className="rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-sm text-warning"
        >
          {t('settings.pinnedDropped', { keys: editor.droppedPinned.join(', ') })}
        </p>
      )}
      <div className="grid grid-cols-1 gap-6 xl:grid-cols-[minmax(0,1fr)_13rem] xl:items-start">
        <CategoryNav
          items={categories.map((category) => ({
            name: category.name,
            label: categoryLabel(category.name, t),
            unsaved: unsavedCount(category, editor.drafts),
          }))}
          onJump={jump}
        />
        <div className="min-w-0 space-y-6 [&_h2]:scroll-mt-22 [&_h2]:outline-none xl:[&_h2]:scroll-mt-10 [&>section]:transition-shadow [&>section:has(h2:focus)]:border-ring [&>section:has(h2:focus)]:ring-3 [&>section:has(h2:focus)]:ring-ring/50">
          {categories.map((category) => (
            <CategoryCard
              key={category.name}
              category={category}
              editor={editor}
              titleRef={register(category.name)}
              onHistory={setHistoryKey}
            />
          ))}
        </div>
      </div>
      <SaveArea settings={settings} editor={editor} />
      <HistoryDrawer
        setting={settings.find((setting) => setting.key === historyKey)}
        onClose={() => {
          setHistoryKey(undefined)
        }}
      />
      <UnsavedGuard dirty={Object.keys(editor.drafts).length > 0} />
    </>
  )
}

interface CategoryCardProps {
  category: Category
  editor: SettingsEditor
  titleRef: RefCallback<HTMLHeadingElement>
  onHistory: (key: string) => void
}

function CategoryCard({ category, editor, titleRef, onHistory }: CategoryCardProps) {
  const { t } = useTranslation()
  return (
    <SectionCard title={categoryLabel(category.name, t)} titleRef={titleRef}>
      <ul className="-my-4 divide-y">
        {category.settings.map((setting) => (
          <SettingRow
            key={setting.key}
            setting={setting}
            draft={editor.drafts[setting.key]}
            typed={editor.typed[setting.key]}
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
              onHistory(setting.key)
            }}
          />
        ))}
      </ul>
    </SectionCard>
  )
}

function SaveArea({
  settings,
  editor,
}: {
  settings: readonly SettingView[]
  editor: SettingsEditor
}) {
  const invalid = settings.some((setting) => {
    const draft = editor.drafts[setting.key]
    return draft?.change.kind === 'set' && problemOf(setting, draft.change.editor) !== undefined
  })
  const explained = editor.conflicts.length > 0 || Object.keys(editor.problems).length > 0
  return (
    <>
      <SaveBar
        count={Object.keys(editor.drafts).length}
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
    </>
  )
}
