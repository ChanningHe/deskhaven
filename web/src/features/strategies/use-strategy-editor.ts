// State of the strategy editor: two rebased drafts over the server copy (name and note,
// and the option map) saved together as one merge patch of the edited fields only.

import { useQueryClient } from '@tanstack/react-query'
import type { SubmitEvent } from 'react'

import { getGetStrategyQueryKey, useUpdateStrategy } from '@/api/generated/strategies/strategies'
import type { CatalogOptionView, StrategyPatch, StrategyView } from '@/api/generated/model'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { useCatalog } from './catalog'
import { invalidateStrategies } from './invalidate'
import { draftOf, patchOf, valueProblem } from './option-draft'
import type { OptionDraft, OptionValue, ValueProblem } from './option-draft'

export interface Details {
  name: string
  note: string
}

/** Problems of the edited options (only those: a stored value is the server's business). */
function problemsOf(
  draft: OptionDraft,
  changed: string[],
  catalog: Map<string, CatalogOptionView>,
) {
  const problems = new Map<string, ValueProblem>()
  for (const key of changed) {
    const value = draft[key]
    if (value?.kind !== 'set') continue
    const problem = valueProblem(catalog.get(key), value.value)
    if (problem !== undefined) problems.set(key, problem)
  }
  return problems
}

function patchFor(
  details: Details,
  detailsChanged: (keyof Details)[],
  options: Record<string, string | null>,
) {
  const patch: StrategyPatch = {}
  if (detailsChanged.includes('name')) patch.name = details.name
  if (detailsChanged.includes('note')) patch.note = details.note
  if (Object.keys(options).length > 0) patch.options = options
  return patch
}

export function useStrategyEditor(strategy: StrategyView) {
  const queryClient = useQueryClient()
  const catalog = useCatalog()
  const details = useRebasedDraft<Details>({ name: strategy.name, note: strategy.note })
  const options = useRebasedDraft<OptionDraft>(draftOf(strategy))
  const mutation = useUpdateStrategy()
  const known = new Map((catalog.data?.options ?? []).map((o) => [o.key, o]))
  const changedKeys = options.changed.map(String)

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const sentOptions = Object.fromEntries(changedKeys.map((k) => [k, options.draft[k]]))
    const sentDetails = Object.fromEntries(details.changed.map((k) => [k, details.draft[k]]))
    const data = patchFor(details.draft, details.changed, patchOf(options.draft, changedKeys))
    const saved = await mutation.mutateAsync({ id: strategy.id, data }).catch(() => undefined)
    if (saved === undefined) return
    // The response is the newest copy: a refetch still in flight must not overwrite it.
    const key = getGetStrategyQueryKey(strategy.id)
    await queryClient.cancelQueries({ queryKey: key })
    queryClient.setQueryData<StrategyView>(key, saved)
    details.reset({ name: saved.name, note: saved.note }, sentDetails)
    options.reset(draftOf(saved), sentOptions)
    void invalidateStrategies(queryClient)
  }

  return {
    catalog,
    details,
    options,
    mutation,
    known,
    problems: problemsOf(options.draft, changedKeys, known),
    dirty: details.changed.length > 0 || changedKeys.length > 0,
    conflicts: details.conflicts.length + options.conflicts.length > 0,
    /** Options outside the catalog that the draft sets. */
    free: Object.entries(options.draft).filter(
      (entry): entry is [string, OptionValue] => entry[1] !== undefined && !known.has(entry[0]),
    ),
    setOption: (key: string, value: OptionValue | undefined) => {
      options.set({ [key]: value })
    },
    submit,
  }
}
