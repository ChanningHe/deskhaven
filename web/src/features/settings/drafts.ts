// Unsaved edits of the settings page: one draft per changed setting, kept as an immutable
// record. A draft remembers the version of the setting it was started from, so the batch
// is saved against what the person actually saw, not against a later background refetch.

import type { SettingChangeBody, SettingView } from '@/api/generated/model'

import { fromEditor, sameEditor, toEditor } from './editor'
import type { Editor } from './editor'

export type DraftChange = { kind: 'set'; editor: Editor } | { kind: 'reset' }

export interface Draft {
  /** `version` of the setting when the edit started. */
  version: number
  change: DraftChange
}

export type Drafts = Readonly<Record<string, Draft>>

export const NO_DRAFTS: Drafts = {}

function without(drafts: Drafts, key: string): Drafts {
  return Object.fromEntries(Object.entries(drafts).filter(([draftKey]) => draftKey !== key))
}

/** `editor` as the new value of `setting`; an edit back to the current value drops the draft. */
export function editSetting(drafts: Drafts, setting: SettingView, editor: Editor): Drafts {
  if (sameEditor(setting, editor, toEditor(setting))) return without(drafts, setting.key)
  const version = drafts[setting.key]?.version ?? setting.version
  return { ...drafts, [setting.key]: { version, change: { kind: 'set', editor } } }
}

/** Marks `setting` to be reset to its default on save. */
export function stageReset(drafts: Drafts, setting: SettingView): Drafts {
  const version = drafts[setting.key]?.version ?? setting.version
  return { ...drafts, [setting.key]: { version, change: { kind: 'reset' } } }
}

export function discardDraft(drafts: Drafts, key: string): Drafts {
  return without(drafts, key)
}

export function discardDrafts(drafts: Drafts, keys: readonly string[]): Drafts {
  return Object.fromEntries(Object.entries(drafts).filter(([key]) => !keys.includes(key)))
}

/** The drafts with the given keys started from the versions in `versions` instead. */
export function rebase(drafts: Drafts, versions: Readonly<Record<string, number>>): Drafts {
  return Object.fromEntries(
    Object.entries(drafts).map(([key, draft]) => [
      key,
      { ...draft, version: versions[key] ?? draft.version },
    ]),
  )
}

/** The request body entries for `drafts`, in the order of `settings`. */
export function toChanges(drafts: Drafts, settings: readonly SettingView[]): SettingChangeBody[] {
  return settings.flatMap((setting): SettingChangeBody[] => {
    const draft = drafts[setting.key]
    if (draft === undefined) return []
    const { key } = setting
    return draft.change.kind === 'reset'
      ? [{ key, version: draft.version, reset: true }]
      : [{ key, version: draft.version, value: fromEditor(setting, draft.change.editor) }]
  })
}
