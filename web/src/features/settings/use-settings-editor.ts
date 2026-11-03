import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'

import { getListSettingsQueryKey, useUpdateSettings } from '@/api/generated/settings/settings'
import type { SettingView } from '@/api/generated/model'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'

import { conflictsOf, fieldProblemsOf } from './conflicts'
import {
  discardDraft,
  discardDrafts,
  editSetting,
  NO_DRAFTS,
  rebase,
  stageReset,
  toChanges,
} from './drafts'
import type { Drafts } from './drafts'
import type { Editor } from './editor'

type Problems = Readonly<Record<string, readonly string[]>>

/** Everything the page does with unsaved edits: stage, save, resolve a conflict. */
export function useSettingsEditor(settings: readonly SettingView[]) {
  const queryClient = useQueryClient()
  // The secrets in the request are dropped from the mutation cache as soon as it settles.
  const mutation = useUpdateSettings(SECRET_MUTATION)
  const [drafts, setDrafts] = useState<Drafts>(NO_DRAFTS)
  const [problems, setProblems] = useState<Problems>({})
  const [conflicts, setConflicts] = useState<SettingView[]>([])
  const [savedCount, setSavedCount] = useState<number | undefined>(undefined)
  const refresh = () => queryClient.invalidateQueries({ queryKey: getListSettingsQueryKey() })

  /** Saves `batch` (the drafts, possibly rebased); the outcome lands in the state below. */
  const run = async (batch: Drafts) => {
    const changes = toChanges(batch, settings)
    setProblems({})
    setSavedCount(undefined)
    try {
      const updated = await mutation.mutateAsync({ data: { changes } })
      queryClient.setQueryData(getListSettingsQueryKey(), updated)
      setDrafts(NO_DRAFTS)
      setSavedCount(changes.length)
      // Drop the request (it may hold a plaintext secret) as soon as it has been saved.
      mutation.reset()
    } catch (error) {
      setConflicts(conflictsOf(error))
      setProblems(fieldProblemsOf(error))
      // A key pinned meanwhile: show what the server has now.
      void refresh()
    }
  }

  return {
    drafts,
    problems,
    conflicts,
    savedCount,
    saving: mutation.isPending,
    error: mutation.error,
    edit: (setting: SettingView, editor: Editor) => {
      setSavedCount(undefined)
      setDrafts((current) => editSetting(current, setting, editor))
    },
    stageReset: (setting: SettingView) => {
      setSavedCount(undefined)
      setDrafts((current) => stageReset(current, setting))
    },
    discard: (key: string) => {
      setDrafts((current) => discardDraft(current, key))
    },
    discardAll: () => {
      setDrafts(NO_DRAFTS)
      setProblems({})
      mutation.reset()
    },
    save: () => run(drafts),
    /** Keep my values: save again against the versions the server has now. */
    overwrite: () => {
      const versions = Object.fromEntries(conflicts.map((c) => [c.key, c.version]))
      setConflicts([])
      return run(rebase(drafts, versions))
    },
    /** Take the server's values: drop my edits of the conflicting keys and reload. */
    reload: () => {
      setDrafts((current) =>
        discardDrafts(
          current,
          conflicts.map((c) => c.key),
        ),
      )
      setConflicts([])
      mutation.reset()
      return refresh()
    },
    dismissConflict: () => {
      setConflicts([])
    },
  }
}
