// State of the strategy editor: two rebased drafts over the server copy (name and note,
// and the option map) saved together as one merge patch of the edited fields only.

import { useQueryClient } from '@tanstack/react-query'
import type { SubmitEvent } from 'react'

import { getGetStrategyQueryKey, useUpdateStrategy } from '@/api/generated/strategies/strategies'
import type { CatalogOptionView, StrategyPatch, StrategyView } from '@/api/generated/model'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { useCatalog } from './catalog'
import { invalidateStrategies } from './invalidate'
import { draftOf, MAX_OPTIONS, optionCount, patchOf, valueProblem } from './option-draft'
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
  formKeys: Set<string>,
) {
  const problems = new Map<string, ValueProblem>()
  for (const key of changed) {
    const value = draft[key]
    if (value?.kind !== 'set') continue
    // "Set to" an empty value would silently mean "reset": the form asks for the reset.
    const problem =
      value.value === '' && formKeys.has(key)
        ? 'emptySet'
        : valueProblem(catalog.get(key), value.value)
    if (problem !== undefined) problems.set(key, problem)
  }
  return problems
}

/** Catalog keys of the typed form (the advanced-only ones are edited as free keys). */
function formKeysOf(options: CatalogOptionView[]): Set<string> {
  return new Set(options.filter((o) => !o.advanced_only).map((o) => o.key))
}

/** Options whose value must not be shown: catalog secrets and withheld saved values. */
function secretKeysOf(options: CatalogOptionView[], strategy: StrategyView): Set<string> {
  const secret = options.filter((o) => o.secret).map((o) => o.key)
  return new Set([...secret, ...strategy.secret_options])
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
  const catalogOptions = catalog.data?.options ?? []
  const known = new Map(catalogOptions.map((o) => [o.key, o]))
  const formKeys = formKeysOf(catalogOptions)
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
    formKeys,
    secretKeys: secretKeysOf(catalogOptions, strategy),
    problems: problemsOf(options.draft, changedKeys, known, formKeys),
    tooMany: optionCount(options.draft) > MAX_OPTIONS,
    dirty: details.changed.length > 0 || changedKeys.length > 0,
    conflicts: details.conflicts.length + options.conflicts.length > 0,
    /** Options outside the typed form that the draft sets. */
    free: Object.entries(options.draft).filter(
      (entry): entry is [string, OptionValue] => entry[1] !== undefined && !formKeys.has(entry[0]),
    ),
    setOption: (key: string, value: OptionValue | undefined) => {
      options.set({ [key]: value })
    },
    submit,
  }
}
