// Shapes the client version distribution for the bar chart.

import type { ClientVersions } from '@/api/generated/model'

export type BarKind = 'version' | 'other' | 'unknown'

export interface Bar {
  /** Stable React key and the identity of the bar. */
  key: string
  kind: BarKind
  /** The version as the device reported it; empty for `other` and `unknown`. */
  version: string
  count: number
  /** Width in percent of the plot: the largest bar is 100. */
  width: number
}

type BarRow = Omit<Bar, 'width'>

/** The listed versions, then "other" and "unknown" when they are not empty. */
export function versionBars(versions: ClientVersions): Bar[] {
  const listed: BarRow[] = versions.versions.map(({ version, count }) => ({
    key: `v:${version}`,
    kind: 'version',
    version,
    count,
  }))
  const rest: BarRow[] = [
    { key: 'other', kind: 'other', version: '', count: versions.other },
    { key: 'unknown', kind: 'unknown', version: '', count: versions.unknown },
  ]
  const rows = [...listed, ...rest.filter((row) => row.count > 0)]
  const largest = Math.max(1, ...rows.map((row) => row.count))
  return rows.map((row) => ({ ...row, width: (row.count / largest) * 100 }))
}
