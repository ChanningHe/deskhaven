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
  const [staged, setDrafts] = useState<Drafts>(NO_DRAFTS)
  const [problems, setProblems] = useState<Problems>({})
  const [conflicts, setConflicts] = useState<SettingView[]>([])
  const [failure, setFailure] = useState<unknown>(undefined)
  const [savedCount, setSavedCount] = useState<number | undefined>(undefined)
  const refresh = () => queryClient.invalidateQueries({ queryKey: getListSettingsQueryKey() })
  // A key the server pinned since it was edited can no longer be saved: its draft is dropped
  // (and the page says so) instead of being sent into a 409.
  const droppedPinned = Object.keys(staged).filter((key) =>
    settings.some((setting) => setting.key === key && setting.pinned),
  )
  const drafts = droppedPinned.length === 0 ? staged : discardDrafts(staged, droppedPinned)

  /** Saves `batch` (the drafts, possibly rebased); the outcome lands in the state below. */
  const run = async (batch: Drafts) => {
    const changes = toChanges(batch, settings)
    setProblems({})
    setFailure(undefined)
    setSavedCount(undefined)
    try {
      const updated = await mutation.mutateAsync({ data: { changes } })
      queryClient.setQueryData(getListSettingsQueryKey(), updated)
      setDrafts(NO_DRAFTS)
      setSavedCount(changes.length)
    } catch (error) {
      setConflicts(conflictsOf(error))
      setProblems(fieldProblemsOf(error))
      setFailure(error)
      // A key pinned meanwhile: show what the server has now.
      void refresh()
    } finally {
      // The request may hold a plaintext secret: the outcome is kept above, the request is not.
      mutation.reset()
    }
  }

  return {
    drafts,
    droppedPinned,
    problems,
    conflicts,
    savedCount,
    saving: mutation.isPending,
    error: failure,
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
      setFailure(undefined)
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
      setFailure(undefined)
      return refresh()
    },
    dismissConflict: () => {
      setConflicts([])
    },
  }
}
