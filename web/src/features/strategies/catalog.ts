// The client option catalog (`GET /strategy-catalog`): fixed for a server build, so it is
// fetched once per session.

import { useGetStrategyCatalog } from '@/api/generated/strategies/strategies'
import type { CatalogOptionView, OptionGroupView, TextView } from '@/api/generated/model'

/** Editor sections, in display order. */
export const GROUP_ORDER: OptionGroupView[] = [
  'security',
  'permissions',
  'display',
  'network',
  'recording',
  'presets',
  'other',
]

export function useCatalog() {
  return useGetStrategyCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } })
}

/** The catalog text in the console language (Chinese for any `zh*`, English otherwise). */
export function textOf(text: TextView, language: string): string {
  return language.startsWith('zh') ? text.zh : text.en
}

/** Catalog options by section, in [`GROUP_ORDER`]; empty sections left out. */
export function grouped(
  options: CatalogOptionView[],
): { group: OptionGroupView; options: CatalogOptionView[] }[] {
  return GROUP_ORDER.map((group) => ({
    group,
    options: options.filter((o) => o.group === group),
  })).filter((section) => section.options.length > 0)
}
